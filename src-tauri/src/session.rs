//! The connection state machine (SPEC §6): discovery, hotplug, watchdog, retries, pause on
//! minimise, recording lifecycle. Runs on one thread; every input arrives as a [`Msg`].

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use camera_control::{CameraInfo, Mode, find_camera};

use crate::audio;
use crate::capture;
use crate::config::{CaptureKind, ConfigStore, VideoQuality};
use crate::controls::{self, Controls};
use crate::pipeline::{PipeEvent, Pipeline, RecordOptions};
use crate::preview::Preview;
use crate::ui::{CamState, ModesPayload, RecordingPayload, StatePayload, Stats, Ui};

const TICK: Duration = Duration::from_millis(100);
const WATCHDOG: Duration = Duration::from_secs(2);
const TEARDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const BUSY_RETRY: Duration = Duration::from_secs(2);
const RESCAN: Duration = Duration::from_secs(2);
const ERROR_BACKOFF: [u64; 4] = [1, 2, 5, 10];
/// After a stop request: push EOS into the muxer directly after this long…
const EOS_FORCE_AFTER: Duration = Duration::from_millis(1500);
/// …and give up on a clean finalize after this long (the Matroska file is still playable).
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const SPACE_CHECK: Duration = Duration::from_secs(3);
/// A mic that hasn't produced audio by now is given up on (the muxer would otherwise hold back
/// the video too). Must stay below the record queue's 3 s so no video is lost.
const AUDIO_START_TIMEOUT: Duration = Duration::from_secs(2);
/// Right after (re)plugging, udev may not have applied the `uaccess` ACL yet (EACCES): retry
/// quietly for this long before showing an error.
const ENUMERATION_GRACE: Duration = Duration::from_secs(3);

pub type Reply<T = ()> = Sender<Result<T, String>>;

pub enum Msg {
    Hotplug,
    Pipe(u64, PipeEvent),
    /// Mic lookup result for the camera at this sysfs path.
    AudioNode(PathBuf, Option<String>),
    SetMode(Mode, Reply),
    CaptureStill(Reply),
    StartRecording {
        audio: bool,
        quality: VideoQuality,
        reply: Reply,
    },
    StopRecording(Reply),
    Minimized(bool),
    Hidden(bool),
    ManualPause(bool),
    Shutdown(Sender<()>),
}

#[derive(Clone)]
pub struct SessionHandle {
    tx: Sender<Msg>,
}

impl SessionHandle {
    pub fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
    }

    pub fn call(&self, timeout: Duration, make: impl FnOnce(Reply) -> Msg) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.send(make(reply));
        rx.recv_timeout(timeout)
            .unwrap_or_else(|_| Err("the camera backend is busy, try again".into()))
    }
}

struct RecState {
    id: u64,
    path: PathBuf,
    started: Instant,
    stop_requested: Option<Instant>,
    forced: bool,
    /// Camera vanished while recording: show the "interrupted" toast.
    interrupted: bool,
    last_space_check: Instant,
    last_progress: Instant,
}

/// A pipeline whose recording is being finalized after the camera vanished. It still holds the
/// device, so reconnecting waits for it.
struct Finalizing {
    pipeline: Pipeline,
    rec: RecState,
}

#[derive(Default)]
struct Visibility {
    minimized: bool,
    hidden: bool,
    manual: bool,
}

impl Visibility {
    /// Minimised/hidden: release the camera completely (LED off, SPEC §6.5). A manual pause only
    /// releases the stream and keeps the controls, so settings can be changed while another app
    /// uses the camera.
    fn wants_full_release(&self) -> bool {
        self.minimized || self.hidden
    }
}

struct Session {
    tx: Sender<Msg>,
    ui: Arc<Ui>,
    config: Arc<ConfigStore>,
    controls: Controls,
    preview: Arc<Preview>,

    state: CamState,
    message: Option<String>,
    camera: Option<CameraInfo>,
    camera_seen_at: Instant,
    /// The controls worker holds the camera's control fd (usable without our stream).
    controls_open: bool,
    mic_lookup_running: bool,
    audio_node: Option<String>,
    modes: Vec<Mode>,
    pipeline: Option<Pipeline>,
    generation: u64,
    rec: Option<RecState>,
    finalizing: Option<Finalizing>,
    teardowns: Vec<(Instant, Receiver<()>)>,
    abandoned: u32,

    connect_deadline: Option<Instant>,
    retry_at: Option<Instant>,
    backoff: usize,
    vis: Visibility,
    next_rec_id: u64,

    stats_at: Instant,
    stats_frames: u64,
}

pub fn spawn(ui: Arc<Ui>, config: Arc<ConfigStore>, controls: Controls, preview: Arc<Preview>) -> SessionHandle {
    let (tx, rx) = mpsc::channel();
    let handle = SessionHandle { tx: tx.clone() };
    std::thread::Builder::new()
        .name("session".into())
        .spawn(move || {
            let mut s = Session {
                tx,
                ui,
                config,
                controls,
                preview,
                state: CamState::NoCamera,
                message: None,
                camera: None,
                camera_seen_at: Instant::now(),
                controls_open: false,
                mic_lookup_running: false,
                audio_node: None,
                modes: vec![],
                pipeline: None,
                generation: 0,
                rec: None,
                finalizing: None,
                teardowns: vec![],
                abandoned: 0,
                connect_deadline: None,
                retry_at: Some(Instant::now()),
                backoff: 0,
                vis: Visibility::default(),
                next_rec_id: 1,
                stats_at: Instant::now(),
                stats_frames: 0,
            };
            s.run(rx);
        })
        .expect("spawn session thread");
    handle
}

impl Session {
    fn run(&mut self, rx: Receiver<Msg>) {
        loop {
            match rx.recv_timeout(TICK) {
                Ok(Msg::Shutdown(done)) => {
                    self.shutdown(&rx);
                    let _ = done.send(());
                    return;
                }
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.tick();
        }
    }

    // --- state ----------------------------------------------------------------------------

    fn set_state(&mut self, state: CamState, message: Option<String>) {
        if self.state == state && self.message == message {
            return;
        }
        tracing::info!(?state, ?message, "camera state");
        self.state = state;
        self.message = message;
        self.publish_state();
    }

    fn publish_state(&self) {
        self.ui.state(StatePayload {
            state: self.state,
            message: self.message.clone(),
            device: self.camera.clone(),
            audio_available: self.audio_node.is_some(),
            manual_pause: self.vis.manual,
            controls_available: self.controls_open,
        });
    }

    fn live_state(&self) -> CamState {
        if self.rec.is_some() {
            CamState::Recording
        } else {
            CamState::Streaming
        }
    }

    fn schedule_retry(&mut self, after: Duration) {
        self.retry_at = Some(Instant::now() + after);
    }

    fn fail(&mut self, message: String) {
        self.teardown_pipeline();
        let delay = ERROR_BACKOFF[self.backoff.min(ERROR_BACKOFF.len() - 1)];
        self.backoff += 1;
        self.set_state(CamState::Error, Some(message));
        self.schedule_retry(Duration::from_secs(delay));
    }

    fn camera_present(&self) -> bool {
        self.camera.as_ref().is_some_and(CameraInfo::is_present)
    }

    // --- messages -------------------------------------------------------------------------

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Hotplug => self.on_hotplug(),
            Msg::Pipe(generation, ev) => self.on_pipe(generation, ev),
            Msg::AudioNode(sysfs, node) => {
                self.mic_lookup_running = false;
                if self.camera.as_ref().is_some_and(|c| c.sysfs_path == sysfs) && node != self.audio_node {
                    tracing::info!(?node, "camera microphone");
                    self.audio_node = node;
                    self.publish_state();
                }
            }
            Msg::SetMode(mode, reply) => {
                let _ = reply.send(self.set_mode(mode));
            }
            Msg::CaptureStill(reply) => {
                let _ = reply.send(self.capture_still());
            }
            Msg::StartRecording { audio, quality, reply } => {
                let _ = reply.send(self.start_recording(audio, quality));
            }
            Msg::StopRecording(reply) => {
                self.stop_recording();
                let _ = reply.send(Ok(()));
            }
            Msg::Minimized(v) => {
                self.vis.minimized = v;
                self.apply_visibility();
            }
            Msg::Hidden(v) => {
                self.vis.hidden = v;
                self.apply_visibility();
            }
            Msg::ManualPause(v) => {
                self.vis.manual = v;
                self.apply_visibility();
                self.publish_state();
            }
            Msg::Shutdown(_) => unreachable!("handled in run()"),
        }
    }

    fn on_hotplug(&mut self) {
        if self.pipeline.is_some() || self.state == CamState::Connecting {
            if !self.camera_present() {
                self.camera_lost("Camera disconnected");
            }
            return;
        }
        match self.state {
            CamState::NoCamera | CamState::Error | CamState::Busy => self.schedule_retry(Duration::ZERO),
            CamState::Paused if !self.vis.wants_full_release() => self.schedule_retry(Duration::ZERO),
            CamState::Paused => {
                // Keep the paused screen honest about what's attached.
                let found = find_camera();
                if found.is_none() && self.camera.is_some() {
                    self.camera = None;
                    self.audio_node = None;
                    self.publish_state();
                } else if found.is_some() && self.camera.is_none() {
                    self.camera = found;
                    self.publish_state();
                }
            }
            _ => {}
        }
    }

    fn on_pipe(&mut self, generation: u64, ev: PipeEvent) {
        if let PipeEvent::RecordingFinalized { id } = ev {
            return self.on_finalized(generation, id);
        }
        if generation != self.generation || self.pipeline.is_none() {
            return;
        }
        match ev {
            PipeEvent::FirstFrame => {
                self.connect_deadline = None;
                self.backoff = 0;
                let s = self.live_state();
                self.set_state(s, None);
            }
            PipeEvent::Error {
                in_recording: true,
                message,
                ..
            } if self.camera_present() => {
                // Disk full, encoder failure, mic trouble: stop the recording, keep previewing.
                if self.rec.as_ref().is_some_and(|r| r.stop_requested.is_none()) {
                    self.ui.error(format!("Recording stopped: {message}"));
                    self.stop_recording();
                }
            }
            PipeEvent::AudioError { message } => {
                if let Some(p) = self.pipeline.as_mut()
                    && self.rec.is_some()
                    && p.end_audio()
                {
                    self.ui.error(format!(
                        "The camera microphone failed ({message}); recording video only"
                    ));
                }
            }
            PipeEvent::Error { busy: true, .. } => {
                // Another app streams; the controls stay usable and apply to its picture.
                self.stop_stream();
                self.set_state(CamState::Busy, None);
                self.publish_state();
                self.schedule_retry(BUSY_RETRY);
            }
            PipeEvent::Error { message, .. } => {
                // camera_lost finalizes a running recording before tearing down.
                if self.camera_present() {
                    self.camera_lost(&message);
                } else {
                    self.camera_lost("Camera disconnected");
                }
            }
            PipeEvent::RecordingFinalized { .. } => unreachable!(),
        }
    }

    fn apply_visibility(&mut self) {
        if self.rec.is_some() {
            // SPEC §6.5: recording continues; pausing happens when it stops.
            return;
        }
        if self.vis.wants_full_release() {
            if self.state != CamState::Paused || self.controls_open {
                // SPEC §6.5: release the device completely so the LED goes off.
                self.teardown_pipeline();
                self.set_state(CamState::Paused, None);
                self.publish_state();
            }
        } else if self.vis.manual {
            if self.pipeline.is_some() || self.state != CamState::Paused || !self.controls_open {
                // Stream released for other apps; try_connect (re)opens just the controls.
                self.stop_stream();
                self.set_state(CamState::Paused, None);
                self.schedule_retry(Duration::ZERO);
            }
        } else if self.state == CamState::Paused {
            // Show black (not static) until the first frame: Connecting.
            self.set_state(CamState::Connecting, None);
            self.schedule_retry(Duration::ZERO);
        }
    }

    // --- connect / teardown ---------------------------------------------------------------

    fn try_connect(&mut self) {
        self.retry_at = None;
        if self.vis.wants_full_release() {
            self.set_state(CamState::Paused, None);
            return;
        }
        if !self.teardowns.is_empty() || self.finalizing.is_some() {
            // The previous pipeline may still hold the device; check again shortly.
            self.schedule_retry(TICK);
            return;
        }
        let Some(info) = find_camera() else {
            if self.camera.take().is_some() {
                self.audio_node = None;
            }
            self.close_controls();
            self.set_state(CamState::NoCamera, None);
            self.schedule_retry(RESCAN);
            return;
        };
        let same_camera = self.camera.as_ref().is_some_and(|c| c.sysfs_path == info.sysfs_path);
        if !same_camera {
            self.audio_node = None;
            self.camera_seen_at = Instant::now();
            self.close_controls();
        }
        self.camera = Some(info.clone());
        // Retrying from Busy/Error and a manual pause keep their screens (no flicker).
        if !matches!(self.state, CamState::Busy | CamState::Error | CamState::Paused) {
            self.set_state(CamState::Connecting, None);
        }

        let modes = match self
            .controls
            .call(Duration::from_millis(2500), |reply| controls::Msg::Open {
                info: info.clone(),
                reply,
            }) {
            Ok(m) => m,
            Err(e) if e == controls::DISCONNECTED || !info.is_present() => {
                self.close_controls();
                self.set_state(CamState::NoCamera, None);
                self.schedule_retry(RESCAN);
                return;
            }
            Err(e) if self.camera_seen_at.elapsed() < ENUMERATION_GRACE => {
                tracing::debug!("open failed right after enumeration, retrying: {e}");
                self.set_state(CamState::Connecting, None);
                self.schedule_retry(Duration::from_millis(250));
                return;
            }
            Err(e) => return self.fail(format!("Could not open the camera: {e}")),
        };
        if !self.controls_open {
            self.controls_open = true;
            self.publish_state();
        }
        if modes.is_empty() {
            return self.fail("The camera offers no MJPEG modes".into());
        }
        let mode = pick_mode(&modes, self.config.get().mode);
        self.modes = modes;
        self.ui.modes(ModesPayload {
            modes: self.modes.clone(),
            usb_speed: info.usb_speed_mbps,
            mode: Some(mode),
        });

        if self.audio_node.is_none() && !self.mic_lookup_running {
            self.spawn_mic_lookup(&info);
        }
        if self.vis.manual {
            // Controls only: the stream stays free for other apps.
            self.set_state(CamState::Paused, None);
            return;
        }

        self.generation += 1;
        let generation = self.generation;
        let tx = self.tx.clone();
        let events = Arc::new(move |generation, ev| {
            let _ = tx.send(Msg::Pipe(generation, ev));
        });
        let preview = self.preview.clone();
        let frames = Arc::new(move |f| preview.offer(f));
        let pipeline = match Pipeline::build(generation, &info.node, mode, events, frames) {
            Ok(p) => p,
            Err(e) => return self.fail(e),
        };
        if let Err(e) = pipeline.play() {
            self.pipeline = Some(pipeline);
            return self.fail(e);
        }
        self.pipeline = Some(pipeline);
        self.stats_at = Instant::now();
        self.stats_frames = 0;
        self.connect_deadline = Some(Instant::now() + CONNECT_TIMEOUT);
    }

    fn spawn_mic_lookup(&mut self, info: &CameraInfo) {
        let Some(card) = info.alsa_card else { return };
        let (tx, sysfs) = (self.tx.clone(), info.sysfs_path.clone());
        self.mic_lookup_running = true;
        std::thread::Builder::new()
            .name("mic-lookup".into())
            .spawn(move || {
                // WirePlumber may create the node a moment after the device enumerates.
                for delay in [0, 1, 2, 4] {
                    std::thread::sleep(Duration::from_secs(delay));
                    if let Some(node) = audio::find_mic_node(card) {
                        let _ = tx.send(Msg::AudioNode(sysfs, Some(node)));
                        return;
                    }
                }
                let _ = tx.send(Msg::AudioNode(sysfs, None));
            })
            .ok();
    }

    /// Release our video stream only; the controls stay open.
    fn stop_stream(&mut self) {
        self.connect_deadline = None;
        if let Some(p) = self.pipeline.take() {
            self.teardowns.push((Instant::now(), p.teardown()));
        }
        self.rec = None;
    }

    fn close_controls(&mut self) {
        if self.controls_open {
            self.controls_open = false;
            self.controls.send(controls::Msg::Close);
            self.publish_state();
        }
    }

    /// Release the camera completely: stream and controls.
    fn teardown_pipeline(&mut self) {
        self.stop_stream();
        self.close_controls();
    }

    /// The camera vanished or stopped delivering frames (SPEC §6.3.3/§6.3.4).
    fn camera_lost(&mut self, reason: &str) {
        self.connect_deadline = None;
        if self.rec.is_some()
            && let (Some(mut p), Some(mut rec)) = (self.pipeline.take(), self.rec.take())
        {
            // Finalize the recording first: EOS through the record branch, then teardown.
            rec.interrupted = true;
            p.stop_recording();
            p.force_eos();
            rec.stop_requested.get_or_insert(Instant::now());
            rec.forced = true;
            self.finalizing = Some(Finalizing { pipeline: p, rec });
        }
        self.teardown_pipeline();
        if self.camera_present() {
            self.fail(reason.to_owned());
        } else {
            self.camera = None;
            self.audio_node = None;
            self.set_state(CamState::NoCamera, None);
            self.schedule_retry(RESCAN);
        }
    }

    // --- capture --------------------------------------------------------------------------

    fn set_mode(&mut self, mode: Mode) -> Result<(), String> {
        if self.rec.is_some() {
            return Err("The mode can't change while recording".into());
        }
        if !self.modes.contains(&mode) {
            return Err("This mode isn't available".into());
        }
        self.config.update(|c| c.mode = Some(mode));
        self.config.save_if_dirty();
        if self.pipeline.as_ref().is_some_and(|p| p.mode == mode) {
            return Ok(());
        }
        if self.pipeline.is_some() || self.state == CamState::Connecting {
            self.stop_stream();
            self.set_state(CamState::Connecting, None);
            self.schedule_retry(Duration::ZERO);
        }
        Ok(())
    }

    fn capture_still(&mut self) -> Result<(), String> {
        let Some(p) = self.pipeline.as_ref().filter(|_| self.state.is_live()) else {
            return Err("The camera isn't streaming".into());
        };
        let rx = p.request_still();
        let dir = self.config.get().photo_dir();
        let ui = self.ui.clone();
        std::thread::Builder::new()
            .name("still".into())
            .spawn(move || match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(jpeg) => match capture::write_still(&dir, &jpeg) {
                    Ok(path) => ui.capture_saved(&path, CaptureKind::Photo, false),
                    Err(e) => ui.error(format!("Couldn't save the photo in {}: {e}", dir.display())),
                },
                // The camera vanished (or stalled) before a complete frame arrived: no file.
                Err(_) => ui.error("Couldn't take the photo: no frame from the camera"),
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn start_recording(&mut self, audio: bool, quality: VideoQuality) -> Result<(), String> {
        if self.rec.is_some() {
            return Err("Already recording".into());
        }
        if !self.state.is_live() || self.pipeline.is_none() {
            return Err("The camera isn't streaming".into());
        }
        let dir = self.config.get().video_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
        let free =
            capture::free_space(&dir).map_err(|e| format!("Can't check free space in {}: {e}", dir.display()))?;
        if free < capture::MIN_FREE_START {
            return Err(format!("Not enough free space in {} (less than 1 GB)", dir.display()));
        }
        let (path, file) =
            capture::reserve(&dir, "mkv").map_err(|e| format!("Can't write to {}: {e}", dir.display()))?;
        drop(file);
        let audio_node = if audio { self.audio_node.clone() } else { None };
        // Debug builds: force a specific (e.g. bogus) mic node to exercise the failure paths.
        #[cfg(debug_assertions)]
        let audio_node = match std::env::var("CAMERA_SETTINGS_DEBUG_AUDIO_NODE") {
            Ok(node) if audio => Some(node),
            _ => audio_node,
        };
        if audio && audio_node.is_none() {
            self.ui.error("Camera microphone not found; recording without audio");
        }
        let id = self.next_rec_id;
        self.next_rec_id += 1;
        let opts = RecordOptions {
            path: path.clone(),
            quality,
            audio_node,
            id,
        };
        let p = self.pipeline.as_mut().unwrap();
        if let Err(e) = p.start_recording(&opts) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        if opts.audio_node.is_some() && !p.has_audio() {
            self.ui
                .error("The camera microphone didn't start; recording video only");
        }
        let now = Instant::now();
        self.rec = Some(RecState {
            id,
            path,
            started: now,
            stop_requested: None,
            forced: false,
            interrupted: false,
            last_space_check: now,
            last_progress: now,
        });
        self.set_state(CamState::Recording, None);
        self.ui.recording(RecordingPayload {
            elapsed_ms: 0,
            bytes: 0,
        });
        Ok(())
    }

    fn stop_recording(&mut self) {
        let (Some(p), Some(rec)) = (self.pipeline.as_mut(), self.rec.as_mut()) else {
            return;
        };
        if rec.stop_requested.is_none() {
            rec.stop_requested = Some(Instant::now());
            p.stop_recording();
        }
    }

    fn on_finalized(&mut self, generation: u64, id: u64) {
        if self
            .finalizing
            .as_ref()
            .is_some_and(|f| f.pipeline.generation == generation && f.rec.id == id)
        {
            let f = self.finalizing.take().unwrap();
            self.finish_finalizing(f);
            return;
        }
        if generation == self.generation && self.rec.as_ref().is_some_and(|r| r.id == id) {
            self.complete_recording();
        }
    }

    /// The record branch of the live pipeline is done (finalized or given up on).
    fn complete_recording(&mut self) {
        let Some(rec) = self.rec.take() else { return };
        if let Some(p) = self.pipeline.as_mut() {
            p.finish_recording();
        }
        self.report_recording(&rec.path, rec.interrupted);
        if self.state == CamState::Recording {
            self.set_state(CamState::Streaming, None);
        }
        // SPEC §6.5: still minimised when recording stops → pause now.
        self.apply_visibility();
    }

    /// "Saved" toast, or, if nothing at all was written, remove the empty file instead of
    /// leaving a zero-byte capture behind.
    fn report_recording(&self, path: &std::path::Path, interrupted: bool) {
        if std::fs::metadata(path).is_ok_and(|m| m.len() == 0) {
            let _ = std::fs::remove_file(path);
            self.ui.error("The recording failed: no data was written");
        } else {
            self.ui.capture_saved(path, CaptureKind::Video, interrupted);
        }
    }

    fn finish_finalizing(&mut self, f: Finalizing) {
        self.report_recording(&f.rec.path, true);
        self.teardowns.push((Instant::now(), f.pipeline.teardown()));
    }

    // --- periodic -------------------------------------------------------------------------

    fn tick(&mut self) {
        let now = Instant::now();

        // Reap finished teardowns; abandon stuck ones (SPEC §6.3.2).
        let mut abandoned = 0;
        self.teardowns.retain(|(since, rx)| match rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => false,
            Err(TryRecvError::Empty) if now.duration_since(*since) < TEARDOWN_TIMEOUT => true,
            Err(TryRecvError::Empty) => {
                abandoned += 1;
                false
            }
        });
        if abandoned > 0 {
            self.abandoned += abandoned;
            tracing::warn!("abandoned a pipeline whose shutdown hung ({} so far)", self.abandoned);
        }

        if let Some(f) = &self.finalizing
            && f.rec
                .stop_requested
                .is_some_and(|t| now.duration_since(t) > FINALIZE_TIMEOUT)
        {
            tracing::warn!("recording finalize timed out after unplug");
            let f = self.finalizing.take().unwrap();
            self.finish_finalizing(f);
        }

        if self.state.is_live()
            && let Some(p) = &self.pipeline
            && p.stats.since_last_frame().is_some_and(|d| d > WATCHDOG)
        {
            tracing::warn!("watchdog: no frame for {WATCHDOG:?}");
            self.camera_lost("The camera stopped sending frames");
        }

        if self.connect_deadline.is_some_and(|t| now > t) {
            if self.camera_present() {
                self.fail("The camera didn't start streaming".into());
            } else {
                self.camera_lost("Camera disconnected");
            }
        }

        if let Some(t) = self.retry_at
            && now >= t
        {
            self.try_connect();
        }

        self.tick_recording(now);
        self.tick_stats(now);
    }

    fn tick_recording(&mut self, now: Instant) {
        let Some(rec) = self.rec.as_mut() else { return };
        if let Some(t) = rec.stop_requested {
            let waited = now.duration_since(t);
            if waited > EOS_FORCE_AFTER && !rec.forced {
                rec.forced = true;
                if let Some(p) = self.pipeline.as_mut() {
                    p.force_eos();
                }
            }
            if waited > FINALIZE_TIMEOUT {
                tracing::warn!("recording finalize timed out");
                self.complete_recording();
            }
            return;
        }
        if now.duration_since(rec.started) > AUDIO_START_TIMEOUT
            && let Some(p) = self.pipeline.as_mut()
            && p.audio_missing()
        {
            p.end_audio();
            self.ui
                .error("The camera microphone isn't delivering audio; recording video only");
        }
        if now.duration_since(rec.last_progress) >= Duration::from_millis(500) {
            rec.last_progress = now;
            let bytes = std::fs::metadata(&rec.path).map(|m| m.len()).unwrap_or(0);
            self.ui.recording(RecordingPayload {
                elapsed_ms: now.duration_since(rec.started).as_millis() as u64,
                bytes,
            });
        }
        if now.duration_since(rec.last_space_check) >= SPACE_CHECK {
            rec.last_space_check = now;
            let dir = rec.path.parent().map(PathBuf::from).unwrap_or_default();
            match capture::free_space(&dir) {
                Ok(free) if free < capture::MIN_FREE_RUNNING => {
                    self.ui.error("Recording stopped: the disk is almost full");
                    self.stop_recording();
                }
                Err(e) if !dir.exists() => {
                    self.ui.error(format!("Recording stopped: {e}"));
                    self.stop_recording();
                }
                _ => {}
            }
        }
    }

    fn tick_stats(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.stats_at);
        if elapsed < Duration::from_secs(1) {
            return;
        }
        let Some(p) = &self.pipeline else { return };
        let frames = p.stats.frames.load(std::sync::atomic::Ordering::Relaxed);
        let fps = (frames - self.stats_frames) as f64 / elapsed.as_secs_f64();
        self.stats_at = now;
        self.stats_frames = frames;
        let bitrate = self.rec.as_ref().and_then(|r| {
            let bytes = std::fs::metadata(&r.path).ok()?.len();
            let secs = now.duration_since(r.started).as_secs_f64();
            (secs > 0.5).then(|| bytes as f64 * 8.0 / secs)
        });
        tracing::debug!(
            mode = format!("{}x{}", p.mode.width, p.mode.height),
            fps = format!("{fps:.1}"),
            preview_sent = self.preview.sent.load(std::sync::atomic::Ordering::Relaxed),
            preview_skipped = self.preview.skipped.load(std::sync::atomic::Ordering::Relaxed),
            "stats"
        );
        self.ui.stats(Stats {
            fps,
            dropped: p.stats.dropped.load(std::sync::atomic::Ordering::Relaxed),
            resolution: format!("{}×{}", p.mode.width, p.mode.height),
            format: format!("MJPEG @ {:.0} fps", p.mode.fps()),
            usb_speed: self.camera.as_ref().map_or(0, |c| c.usb_speed_mbps),
            bitrate,
        });
    }

    fn shutdown(&mut self, rx: &Receiver<Msg>) {
        // Finalize a running recording so the file is complete, bounded by the finalize timeout.
        // Pipeline events must still be handled: they carry the "finalized" notification.
        self.stop_recording();
        let deadline = Instant::now() + FINALIZE_TIMEOUT + TICK;
        while self.rec.is_some() && Instant::now() < deadline {
            if let Ok(Msg::Pipe(generation, ev)) = rx.recv_timeout(TICK) {
                self.on_pipe(generation, ev);
            }
            self.tick_recording(Instant::now());
        }
        self.teardown_pipeline();
        if let Some(f) = self.finalizing.take() {
            self.teardowns.push((Instant::now(), f.pipeline.teardown()));
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        for (_, rx) in self.teardowns.drain(..) {
            let _ = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        }
        self.config.save_if_dirty();
    }
}

/// Saved mode if the camera offers it now (USB 2 has fewer), else 1080p30, else the largest ≤ 1080p.
fn pick_mode(modes: &[Mode], saved: Option<Mode>) -> Mode {
    saved
        .filter(|m| modes.contains(m))
        .or_else(|| modes.iter().copied().find(|m| *m == Mode::DEFAULT))
        .or_else(|| modes.iter().copied().find(|m| m.height <= 1080))
        .unwrap_or(modes[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(width: u32, height: u32, fps: u32) -> Mode {
        Mode {
            width,
            height,
            fps_n: fps,
            fps_d: 1,
        }
    }

    #[test]
    fn mode_selection() {
        let usb3 = [
            m(3840, 2160, 30),
            m(1920, 1080, 60),
            m(1920, 1080, 30),
            m(1280, 720, 90),
        ];
        assert_eq!(pick_mode(&usb3, None), m(1920, 1080, 30));
        assert_eq!(pick_mode(&usb3, Some(m(3840, 2160, 30))), m(3840, 2160, 30));
        let usb2 = [m(1920, 1080, 30), m(1280, 720, 30)];
        assert_eq!(pick_mode(&usb2, Some(m(3840, 2160, 30))), m(1920, 1080, 30));
        let odd = [m(2560, 1440, 30), m(1280, 720, 60)];
        assert_eq!(pick_mode(&odd, None), m(1280, 720, 60));
    }
}
