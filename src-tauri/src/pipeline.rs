//! GStreamer pipeline (SPEC §8.1/§8.2):
//!
//! ```text
//! v4l2src (MJPEG) → capsfilter → tee ─┬→ queue(leaky,1) → [decode→scale→jpegenc if >1080p] → appsink  (preview)
//!                    ▲ probe:         └→ record bin: queue → [decode→AV1] → matroskamux → filesink
//!                    stats, watchdog,     pipewiresrc(BRIO mic) → opusenc ↗
//!                    still tap
//! ```

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use camera_control::Mode;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use crate::config::VideoQuality;

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

fn now_ms() -> u64 {
    // +1 so that 0 can mean "never".
    EPOCH.elapsed().as_millis() as u64 + 1
}

pub fn init() -> Result<(), String> {
    gst::init().map_err(|e| format!("GStreamer init failed: {e}"))?;
    LazyLock::force(&EPOCH);
    Ok(())
}

fn has(factory: &str) -> bool {
    gst::ElementFactory::find(factory).is_some()
}

/// Is any AV1 encoder available ("Compact" quality is hidden otherwise)?
pub fn av1_encoder() -> Option<&'static str> {
    ["vaav1enc", "svtav1enc"].into_iter().find(|f| has(f))
}

/// `vaav1enc` (Mesa radeonsi) pads frames it can't encode exactly and doesn't signal a crop:
/// 1920×1080 comes out as 1920×1082 with two black rows, 800×600 as 832×602. Measured: the size
/// is kept only for widths that are multiples of 64 and heights that are multiples of 16, from
/// 320×192 up.
fn hw_av1_keeps_size(width: u32, height: u32) -> bool {
    width.is_multiple_of(64) && height.is_multiple_of(16) && width >= 320 && height >= 192
}

/// AV1 encoder for a Compact recording of `width`×`height` (SPEC §5.2): hardware when it keeps
/// the size, else `svtav1enc` (≈1.3 cores at 1080p30), else padded hardware output as a last resort.
fn av1_encoder_for(width: u32, height: u32) -> Option<&'static str> {
    if has("vaav1enc") && hw_av1_keeps_size(width, height) {
        Some("vaav1enc")
    } else if has("svtav1enc") {
        Some("svtav1enc")
    } else {
        av1_encoder()
    }
}

/// Software decode on purpose: `vajpegdec` on the Radeon 890M hard-hangs the GPU ("context is
/// lost … guilty of a hard recovery") on the BRIO's 4:2:2 MJPEG. `jpegdec` handles 4K30 → 1080p
/// preview at ~45% of one core.
const JPEG_DECODER: &str = "jpegdec";

/// Events from GStreamer threads to the session. Tagged with the pipeline generation so late
/// events from a pipeline that was already torn down can be ignored.
#[derive(Debug)]
pub enum PipeEvent {
    FirstFrame,
    Error {
        busy: bool,
        in_recording: bool,
        message: String,
    },
    /// The recording's mic branch failed; the rest of the recording is fine.
    AudioError {
        message: String,
    },
    RecordingFinalized {
        id: u64,
    },
}

pub type EventSink = Arc<dyn Fn(u64, PipeEvent) + Send + Sync>;
pub type FrameSink = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

#[derive(Default)]
pub struct FrameStats {
    pub frames: AtomicU64,
    pub dropped: AtomicU64,
    last_seq: AtomicU64,
    last_frame_ms: AtomicU64,
}

impl FrameStats {
    /// Time since the last camera frame, or `None` before the first one.
    pub fn since_last_frame(&self) -> Option<Duration> {
        match self.last_frame_ms.load(Ordering::Relaxed) {
            0 => None,
            t => Some(Duration::from_millis(now_ms().saturating_sub(t))),
        }
    }
}

type StillRequest = Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>;

pub struct Pipeline {
    pub generation: u64,
    pub mode: Mode,
    pipeline: gst::Pipeline,
    src: gst::Element,
    tee: gst::Element,
    pub stats: Arc<FrameStats>,
    still: StillRequest,
    bus_stop: Arc<AtomicBool>,
    events: EventSink,
    recording: Option<Recording>,
    torn_down: bool,
}

pub struct RecordOptions {
    pub path: PathBuf,
    pub quality: VideoQuality,
    /// PipeWire `node.name` of the mic, or `None` for video only.
    pub audio_node: Option<String>,
    pub id: u64,
}

struct Recording {
    bin: gst::Bin,
    mux: gst::Element,
    tee_pad: gst::Pad,
    ghost: gst::Pad,
    audio: Option<AudioBranch>,
    stopping: bool,
}

/// The mic lives in its own bin: a PipeWire node that never starts (seen after many replugs)
/// must not hold up the state change of the video half of the recording.
struct AudioBranch {
    bin: gst::Bin,
    src: gst::Element,
    /// The record bin's `audio` ghost pad, i.e. the muxer's audio input.
    mux_pad: gst::Pad,
    seen: Arc<AtomicBool>,
    ended: bool,
}

fn make(factory: &str) -> Result<gst::Element, String> {
    gst::ElementFactory::make(factory)
        .build()
        .map_err(|_| format!("GStreamer element `{factory}` is missing"))
}

fn even(v: u32) -> u32 {
    v & !1
}

impl Pipeline {
    pub fn build(
        generation: u64,
        node: &Path,
        mode: Mode,
        events: EventSink,
        frames: FrameSink,
    ) -> Result<Self, String> {
        let pipeline = gst::Pipeline::with_name(&format!("camera-{generation}"));
        let src = gst::ElementFactory::make("v4l2src")
            .name("src")
            .property("device", node.to_string_lossy().as_ref())
            .build()
            .map_err(|_| "GStreamer element `v4l2src` is missing".to_string())?;
        let caps = gst::Caps::builder("image/jpeg")
            .field("width", mode.width as i32)
            .field("height", mode.height as i32)
            .field("framerate", gst::Fraction::new(mode.fps_n as i32, mode.fps_d as i32))
            .build();
        let filter = gst::ElementFactory::make("capsfilter")
            .property("caps", &caps)
            .build()
            .map_err(|e| e.to_string())?;
        let tee = gst::ElementFactory::make("tee")
            .name("tee")
            .property("allow-not-linked", true)
            .build()
            .map_err(|e| e.to_string())?;
        let queue = gst::ElementFactory::make("queue")
            .property_from_str("leaky", "downstream")
            .property("max-size-buffers", 1u32)
            .property("max-size-bytes", 0u32)
            .property("max-size-time", 0u64)
            .build()
            .map_err(|e| e.to_string())?;
        let sink = gst_app::AppSink::builder()
            .name("preview")
            .drop(true)
            .max_buffers(1)
            .sync(false)
            .build();
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if let Some(buf) = sample.buffer()
                        && let Ok(map) = buf.map_readable()
                    {
                        frames(map.as_slice().to_vec());
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        pipeline
            .add_many([&src, &filter, &tee, &queue])
            .map_err(|e| e.to_string())?;
        pipeline
            .add(sink.upcast_ref::<gst::Element>())
            .map_err(|e| e.to_string())?;
        gst::Element::link_many([&src, &filter, &tee, &queue]).map_err(|e| e.to_string())?;

        if mode.width > 1920 {
            // Decode and shrink large modes: the webview can't decode 4K MJPEG at 30 fps.
            let dec = make(JPEG_DECODER)?;
            let convert = make("videoconvert")?;
            let scale = make("videoscale")?;
            let height = even((u64::from(mode.height) * 1920 / u64::from(mode.width)) as u32);
            let scaled = gst::ElementFactory::make("capsfilter")
                .property(
                    "caps",
                    gst::Caps::builder("video/x-raw")
                        .field("width", 1920)
                        .field("height", height as i32)
                        .build(),
                )
                .build()
                .map_err(|e| e.to_string())?;
            let enc = gst::ElementFactory::make("jpegenc")
                .property("quality", 85i32)
                .build()
                .map_err(|e| e.to_string())?;
            pipeline
                .add_many([&dec, &convert, &scale, &scaled, &enc])
                .map_err(|e| e.to_string())?;
            gst::Element::link_many([&queue, &dec, &convert, &scale, &scaled, &enc, sink.upcast_ref()])
                .map_err(|e| e.to_string())?;
        } else {
            queue.link(&sink).map_err(|e| e.to_string())?;
        }

        let stats = Arc::new(FrameStats::default());
        let still: StillRequest = Arc::default();
        install_frame_probe(&filter, generation, stats.clone(), still.clone(), events.clone());

        let bus_stop = Arc::new(AtomicBool::new(false));
        spawn_bus_thread(&pipeline, generation, bus_stop.clone(), events.clone());

        Ok(Self {
            generation,
            mode,
            pipeline,
            src,
            tee,
            stats,
            still,
            bus_stop,
            events,
            recording: None,
            torn_down: false,
        })
    }

    pub fn play(&self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Playing)
            .map(drop)
            .map_err(|_| "could not start the camera stream".into())
    }

    /// Ask for the next camera frame's original JPEG bytes.
    pub fn request_still(&self) -> Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        *self.still.lock().unwrap() = Some(tx);
        rx
    }

    pub fn start_recording(&mut self, opts: &RecordOptions) -> Result<(), String> {
        if self.recording.is_some() {
            return Err("already recording".into());
        }
        let bin = gst::Bin::with_name(&format!("record-{}", opts.id));
        let vqueue = gst::ElementFactory::make("queue")
            .property_from_str("leaky", "downstream")
            .property("max-size-buffers", 0u32)
            .property("max-size-bytes", 0u32)
            .property("max-size-time", 3_000_000_000u64)
            .build()
            .map_err(|e| e.to_string())?;
        let mux = gst::ElementFactory::make("matroskamux")
            .property("offset-to-zero", true)
            .build()
            .map_err(|_| "GStreamer element `matroskamux` is missing".to_string())?;
        let filesink = gst::ElementFactory::make("filesink")
            .property("location", opts.path.to_string_lossy().as_ref())
            .property("async", false)
            .property("sync", false)
            .build()
            .map_err(|e| e.to_string())?;
        bin.add_many([&vqueue, &mux, &filesink]).map_err(|e| e.to_string())?;

        let mut video_chain = vec![vqueue.clone()];
        if opts.quality == VideoQuality::Compact {
            let enc_name = av1_encoder_for(self.mode.width, self.mode.height).ok_or("no AV1 encoder available")?;
            tracing::info!("compact recording with {enc_name}");
            let dec = make(JPEG_DECODER)?;
            let convert = make("videoconvert")?;
            // ~0.08 bits per pixel: ≈5 Mbit/s at 1080p30, ≈20 Mbit/s at 4K30.
            let kbps =
                (f64::from(self.mode.width) * f64::from(self.mode.height) * self.mode.fps() * 0.08 / 1000.0) as u32;
            let enc = match enc_name {
                "vaav1enc" => gst::ElementFactory::make(enc_name).property("bitrate", kbps).build(),
                _ => gst::ElementFactory::make(enc_name)
                    .property("target-bitrate", kbps)
                    .property("preset", 10u32)
                    .build(),
            }
            .map_err(|_| format!("GStreamer element `{enc_name}` is missing"))?;
            let parse = make("av1parse")?;
            bin.add_many([&dec, &convert, &enc, &parse])
                .map_err(|e| e.to_string())?;
            video_chain.extend([dec, convert, enc, parse]);
        }
        gst::Element::link_many(&video_chain).map_err(|e| e.to_string())?;
        video_chain
            .last()
            .unwrap()
            .link_pads(None, &mux, Some("video_%u"))
            .map_err(|e| e.to_string())?;
        mux.link(&filesink).map_err(|e| e.to_string())?;

        // The muxer's audio input is requested now so it waits for audio from the start.
        let audio_mux_pad = match &opts.audio_node {
            Some(_) => {
                let pad = mux
                    .request_pad_simple("audio_%u")
                    .ok_or("matroskamux has no audio input")?;
                let ghost = gst::GhostPad::builder_with_target(&pad)
                    .map_err(|e| e.to_string())?
                    .name("audio")
                    .build();
                bin.add_pad(&ghost).map_err(|e| e.to_string())?;
                Some(ghost.upcast::<gst::Pad>())
            }
            None => None,
        };

        let target = vqueue.static_pad("sink").unwrap();
        let ghost = gst::GhostPad::builder_with_target(&target)
            .map_err(|e| e.to_string())?
            .name("sink")
            .build();
        bin.add_pad(&ghost).map_err(|e| e.to_string())?;

        let (events, generation, id) = (self.events.clone(), self.generation, opts.id);
        filesink
            .static_pad("sink")
            .unwrap()
            .add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
                if let Some(gst::PadProbeData::Event(ev)) = &info.data
                    && ev.type_() == gst::EventType::Eos
                {
                    events(generation, PipeEvent::RecordingFinalized { id });
                }
                gst::PadProbeReturn::Ok
            });

        self.pipeline.add(&bin).map_err(|e| e.to_string())?;
        let tee_pad = self.tee.request_pad_simple("src_%u").ok_or("tee has no free pad")?;
        let rollback = |p: &Self, bin: &gst::Bin, tee_pad: &gst::Pad| {
            p.tee.release_request_pad(tee_pad);
            let _ = bin.set_state(gst::State::Null);
            let _ = p.pipeline.remove(bin);
        };
        if let Err(e) = bin.sync_state_with_parent() {
            rollback(self, &bin, &tee_pad);
            return Err(format!("could not start recording: {e}"));
        }
        if let Err(e) = tee_pad.link(ghost.upcast_ref::<gst::Pad>()) {
            rollback(self, &bin, &tee_pad);
            return Err(format!("could not start recording: {e}"));
        }
        let audio = match (&opts.audio_node, audio_mux_pad) {
            (Some(node), Some(mux_pad)) => match self.start_audio(opts.id, node, &mux_pad) {
                Ok(a) => Some(a),
                Err(e) => {
                    tracing::warn!("recording without audio: {e}");
                    drop_mux_input(&bin, &mux, &mux_pad);
                    None
                }
            },
            _ => None,
        };
        self.recording = Some(Recording {
            bin,
            mux,
            tee_pad,
            ghost: ghost.upcast(),
            audio,
            stopping: false,
        });
        Ok(())
    }

    fn start_audio(&self, id: u64, node: &str, mux_pad: &gst::Pad) -> Result<AudioBranch, String> {
        let bin = gst::Bin::with_name(&format!("record-audio-{id}"));
        let src = gst::ElementFactory::make("pipewiresrc")
            .property("target-object", node)
            .property("client-name", "Camera Settings")
            // SPEC §5.2: never the system default mic. Without these, PipeWire silently falls
            // back to the default source when the camera's node isn't there.
            .property(
                "stream-properties",
                gst::Structure::builder("props")
                    .field("node.dont-fallback", true)
                    .field("node.dont-reconnect", true)
                    .build(),
            )
            .build()
            .map_err(|_| "GStreamer element `pipewiresrc` is missing".to_string())?;
        let convert = make("audioconvert")?;
        let resample = make("audioresample")?;
        let caps = gst::ElementFactory::make("capsfilter")
            .property("caps", gst::Caps::builder("audio/x-raw").field("rate", 48_000).build())
            .build()
            .map_err(|e| e.to_string())?;
        let enc = gst::ElementFactory::make("opusenc")
            .property("bitrate", 128_000i32)
            .build()
            .map_err(|_| "GStreamer element `opusenc` is missing".to_string())?;
        let queue = make("queue")?;
        let chain = [&src, &convert, &resample, &caps, &enc, &queue];
        bin.add_many(chain).map_err(|e| e.to_string())?;
        gst::Element::link_many(chain).map_err(|e| e.to_string())?;
        let out = gst::GhostPad::builder_with_target(&queue.static_pad("src").unwrap())
            .map_err(|e| e.to_string())?
            .name("src")
            .build();
        bin.add_pad(&out).map_err(|e| e.to_string())?;

        let seen = Arc::new(AtomicBool::new(false));
        let seen2 = seen.clone();
        src.static_pad("src")
            .unwrap()
            .add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                seen2.store(true, Ordering::Relaxed);
                gst::PadProbeReturn::Remove
            });

        self.pipeline.add(&bin).map_err(|e| e.to_string())?;
        let started = out
            .link(mux_pad)
            .map_err(|e| e.to_string())
            .and_then(|_| bin.sync_state_with_parent().map_err(|e| e.to_string()));
        if let Err(e) = started {
            remove_bin_async(&self.pipeline, bin);
            return Err(e);
        }
        Ok(AudioBranch {
            bin,
            src,
            mux_pad: mux_pad.clone(),
            seen,
            ended: false,
        })
    }

    /// Recording with a mic that hasn't delivered a single buffer yet.
    pub fn audio_missing(&self) -> bool {
        self.recording
            .as_ref()
            .and_then(|r| r.audio.as_ref())
            .is_some_and(|a| !a.ended && !a.seen.load(Ordering::Relaxed))
    }

    /// Give up on the mic: end the audio track so the muxer writes video on its own.
    /// Returns false if there was no live audio branch to end.
    pub fn end_audio(&mut self) -> bool {
        let Some(rec) = self.recording.as_mut() else {
            return false;
        };
        let Some(audio) = rec.audio.as_mut().filter(|a| !a.ended) else {
            return false;
        };
        audio.ended = true;
        if audio.seen.load(Ordering::Relaxed) {
            // The track exists: end it so the file is finalized cleanly.
            audio.mux_pad.send_event(gst::event::Eos::new());
        } else {
            // No audio ever arrived: an input that ends before it had caps keeps matroskamux
            // from writing anything, so remove the input and record video only.
            drop_mux_input(&rec.bin, &rec.mux, &audio.mux_pad);
        }
        remove_bin_async(&self.pipeline, audio.bin.clone());
        true
    }

    /// The running recording has a (started) mic branch.
    pub fn has_audio(&self) -> bool {
        self.recording.as_ref().is_some_and(|r| r.audio.is_some())
    }

    /// Detach the record branch and send EOS through it so Matroska is finalized. Completion is
    /// reported as [`PipeEvent::RecordingFinalized`].
    pub fn stop_recording(&mut self) {
        let Some(rec) = &mut self.recording else { return };
        if rec.stopping {
            return;
        }
        rec.stopping = true;
        let (tee, ghost) = (self.tee.clone(), rec.ghost.clone());
        rec.tee_pad.add_probe(gst::PadProbeType::IDLE, move |pad, _| {
            let _ = pad.unlink(&ghost);
            ghost.send_event(gst::event::Eos::new());
            tee.release_request_pad(pad);
            gst::PadProbeReturn::Remove
        });
        if let Some(audio) = rec.audio.as_ref().filter(|a| !a.ended) {
            audio.src.send_event(gst::event::Eos::new());
        }
    }

    /// Fallback when EOS can't flow normally (e.g. the mic vanished with the camera): push EOS
    /// straight into the muxer inputs.
    pub fn force_eos(&mut self) {
        self.end_audio();
        if let Some(rec) = &self.recording {
            rec.ghost.send_event(gst::event::Eos::new());
        }
    }

    /// Remove a finished (or abandoned) record bin, off the calling thread.
    pub fn finish_recording(&mut self) {
        let Some(rec) = self.recording.take() else { return };
        let pipeline = self.pipeline.clone();
        if !rec.stopping {
            self.tee.release_request_pad(&rec.tee_pad);
        }
        if let Some(audio) = rec.audio.filter(|a| !a.ended) {
            remove_bin_async(&pipeline, audio.bin);
        }
        remove_bin_async(&pipeline, rec.bin);
    }

    /// Stop the pipeline on a dedicated thread (SPEC §6.3.2). The receiver fires when
    /// `set_state(Null)` returned; if it never does, the pipeline is simply leaked.
    pub fn teardown(mut self) -> Receiver<()> {
        self.torn_down = true;
        self.bus_stop.store(true, Ordering::Relaxed);
        *self.still.lock().unwrap() = None;
        let (tx, rx) = mpsc::channel();
        self.stop_in_background(Some(tx));
        rx
    }

    /// Stop everything on a helper thread. The camera is released first and mic bins last and
    /// separately: a wedged `pipewiresrc` can block its state change indefinitely, and must not
    /// keep the camera's fd open (the next connect would then see the device as busy).
    fn stop_in_background(&mut self, done: Option<mpsc::Sender<()>>) {
        let rec = self.recording.take();
        let (pipeline, src) = (self.pipeline.clone(), self.src.clone());
        let audio_bins: Vec<gst::Element> = pipeline
            .children()
            .into_iter()
            .filter(|e| e.name().starts_with("record-audio-"))
            .collect();
        for bin in &audio_bins {
            bin.set_locked_state(true);
        }
        std::thread::Builder::new()
            .name(format!("teardown-{}", self.generation))
            .spawn(move || {
                let _ = src.set_state(gst::State::Null);
                let _ = pipeline.set_state(gst::State::Null);
                if let Some(tx) = done {
                    let _ = tx.send(());
                }
                for bin in audio_bins {
                    let _ = bin.set_state(gst::State::Null);
                }
                drop(rec);
            })
            .ok();
    }
}

impl Drop for Pipeline {
    /// Backstop: disposing a PLAYING pipeline crashes inside GStreamer's streaming threads, so a
    /// pipeline dropped without [`Pipeline::teardown`] is still stopped (on its own thread).
    fn drop(&mut self) {
        if !self.torn_down {
            tracing::error!("pipeline {} dropped without teardown", self.generation);
            self.bus_stop.store(true, Ordering::Relaxed);
            self.stop_in_background(None);
        }
    }
}

/// Remove a muxer input (exposed on `bin` as ghost pad `ghost`) that never received data.
fn drop_mux_input(bin: &gst::Bin, mux: &gst::Element, ghost: &gst::Pad) {
    let target = ghost.downcast_ref::<gst::GhostPad>().and_then(|g| g.target());
    let _ = bin.remove_pad(ghost);
    if let Some(pad) = target {
        mux.release_request_pad(&pad);
    }
}

/// Stop a bin and remove it from the pipeline on a helper thread (stopping `pipewiresrc` can
/// block for a long time when its node is wedged).
fn remove_bin_async(pipeline: &gst::Pipeline, bin: gst::Bin) {
    let pipeline = pipeline.clone();
    std::thread::Builder::new()
        .name("bin-teardown".into())
        .spawn(move || {
            let _ = bin.set_state(gst::State::Null);
            let _ = pipeline.remove(&bin);
        })
        .ok();
}

fn install_frame_probe(
    el: &gst::Element,
    generation: u64,
    stats: Arc<FrameStats>,
    still: StillRequest,
    events: EventSink,
) {
    let pad = el.static_pad("src").expect("capsfilter src pad");
    pad.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
        let Some(gst::PadProbeData::Buffer(buf)) = &info.data else {
            return gst::PadProbeReturn::Ok;
        };
        if stats.frames.fetch_add(1, Ordering::Relaxed) == 0 {
            events(generation, PipeEvent::FirstFrame);
        }
        stats.last_frame_ms.store(now_ms(), Ordering::Relaxed);
        // v4l2src puts the V4L2 sequence number in the buffer offset; gaps are dropped frames.
        let seq = buf.offset();
        if seq != u64::MAX {
            let last = stats.last_seq.swap(seq + 1, Ordering::Relaxed);
            if last != 0 && seq + 1 > last {
                stats.dropped.fetch_add(seq + 1 - last - 1, Ordering::Relaxed);
            }
        }
        if !buf.flags().contains(gst::BufferFlags::CORRUPTED) {
            let mut req = still.lock().unwrap();
            if req.is_some()
                && let Ok(map) = buf.map_readable()
                && is_complete_jpeg(map.as_slice())
            {
                let _ = req.take().unwrap().send(map.as_slice().to_vec());
            }
        }
        gst::PadProbeReturn::Ok
    });
}

/// SOI at the start and EOI at the end (ignoring trailing zero padding some UVC cameras add).
pub fn is_complete_jpeg(data: &[u8]) -> bool {
    let end = data.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    data.len() > 4 && data.starts_with(&[0xff, 0xd8]) && data[..end].ends_with(&[0xff, 0xd9])
}

fn spawn_bus_thread(pipeline: &gst::Pipeline, generation: u64, stop: Arc<AtomicBool>, events: EventSink) {
    let bus = pipeline.bus().expect("pipeline bus");
    let weak = pipeline.downgrade();
    std::thread::Builder::new()
        .name(format!("bus-{generation}"))
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(200)) else {
                    continue;
                };
                match msg.view() {
                    gst::MessageView::Error(err) => {
                        let src_path = msg.src().map(|s| s.path_string().to_string()).unwrap_or_default();
                        let busy = err.error().matches(gst::ResourceError::Busy);
                        tracing::warn!(
                            "pipeline {generation} error from {src_path}: {} ({:?})",
                            err.error(),
                            err.debug()
                        );
                        let message = err.error().to_string();
                        // Path segments look like "/GstBin:record-audio-1/GstPipeWireSrc:pipewiresrc0".
                        let event = if src_path.contains(":record-audio-") {
                            PipeEvent::AudioError { message }
                        } else {
                            PipeEvent::Error {
                                busy,
                                in_recording: src_path.contains(":record-"),
                                message,
                            }
                        };
                        events(generation, event);
                    }
                    gst::MessageView::Warning(w) => {
                        tracing::debug!("pipeline {generation} warning: {} ({:?})", w.error(), w.debug());
                    }
                    gst::MessageView::Latency(_) => {
                        if let Some(p) = weak.upgrade() {
                            let _ = p.recalculate_latency();
                        }
                    }
                    _ => {}
                }
            }
        })
        .expect("spawn bus thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_av1_sizes() {
        // Measured on the Radeon 890M (see hw_av1_keeps_size).
        for (w, h) in [
            (3840, 2160),
            (4096, 2160),
            (2560, 1440),
            (1600, 896),
            (1280, 720),
            (640, 480),
        ] {
            assert!(hw_av1_keeps_size(w, h), "{w}x{h}");
        }
        for (w, h) in [(1920, 1080), (960, 540), (800, 600), (640, 360), (480, 270), (160, 120)] {
            assert!(!hw_av1_keeps_size(w, h), "{w}x{h}");
        }
    }

    #[test]
    fn jpeg_completeness() {
        assert!(is_complete_jpeg(&[0xff, 0xd8, 1, 2, 0xff, 0xd9]));
        assert!(is_complete_jpeg(&[0xff, 0xd8, 1, 2, 0xff, 0xd9, 0, 0]));
        assert!(!is_complete_jpeg(&[0xff, 0xd8, 1, 2, 3]));
        assert!(!is_complete_jpeg(&[]));
    }
}
