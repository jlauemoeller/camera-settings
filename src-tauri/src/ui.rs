//! Everything sent to the frontend: event payloads, and a cached snapshot so `get_snapshot`
//! answers without touching the device.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use camera_control::{CameraInfo, Control, Mode};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::config::{self, CaptureKind, Config, VideoQuality};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CamState {
    NoCamera,
    Connecting,
    Streaming,
    Recording,
    Paused,
    Busy,
    Error,
}

impl CamState {
    pub fn is_live(self) -> bool {
        matches!(self, CamState::Streaming | CamState::Recording)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatePayload {
    pub state: CamState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub device: Option<CameraInfo>,
    pub audio_available: bool,
    /// True when the user paused manually (as opposed to window minimised).
    pub manual_pause: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModesPayload {
    pub modes: Vec<Mode>,
    pub usb_speed: u32,
    pub mode: Option<Mode>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingPayload {
    pub elapsed_ms: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSaved {
    pub path: PathBuf,
    pub kind: CaptureKind,
    pub interrupted: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub fps: f64,
    pub dropped: u64,
    pub resolution: String,
    pub format: String,
    pub usb_speed: u32,
    /// Recording bitrate in bits/s, when recording.
    pub bitrate: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    pub photo_dir: PathBuf,
    pub video_dir: PathBuf,
    pub photo_dir_is_default: bool,
    pub video_dir_is_default: bool,
    pub video_quality: VideoQuality,
    pub audio: bool,
    pub capture_kind: CaptureKind,
}

impl From<&Config> for Prefs {
    fn from(c: &Config) -> Self {
        Prefs {
            photo_dir: c.photo_dir(),
            video_dir: c.video_dir(),
            photo_dir_is_default: c.photo_dir.is_none() || c.photo_dir() == config::default_photo_dir(),
            video_dir_is_default: c.video_dir.is_none() || c.video_dir() == config::default_video_dir(),
            video_quality: c.video_quality,
            audio: c.audio,
            capture_kind: c.capture_kind,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(flatten)]
    pub state: StatePayload,
    pub modes: ModesPayload,
    pub controls: Vec<Control>,
    pub prefs: Prefs,
    pub compact_available: bool,
    pub recording: Option<RecordingPayload>,
    /// Started with `--debug`: keep the webview context menu (web inspector).
    pub debug: bool,
}

/// Emits events and mirrors them into the snapshot.
pub struct Ui {
    app: AppHandle,
    snap: Mutex<Snapshot>,
}

impl Ui {
    pub fn new(app: AppHandle, cfg: &Config, compact_available: bool, debug: bool) -> Self {
        let snap = Snapshot {
            state: StatePayload {
                state: CamState::NoCamera,
                message: None,
                device: None,
                audio_available: false,
                manual_pause: false,
            },
            modes: ModesPayload {
                modes: vec![],
                usb_speed: 0,
                mode: None,
            },
            controls: vec![],
            prefs: Prefs::from(cfg),
            compact_available,
            recording: None,
            debug,
        };
        Self {
            app,
            snap: Mutex::new(snap),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snap.lock().unwrap().clone()
    }

    fn emit<T: Serialize + Clone>(&self, event: &str, payload: T) {
        if let Err(e) = self.app.emit(event, payload) {
            tracing::warn!("emit {event}: {e}");
        }
    }

    pub fn state(&self, p: StatePayload) {
        {
            let mut s = self.snap.lock().unwrap();
            if p.state != CamState::Recording {
                s.recording = None;
            }
            s.state = p.clone();
        }
        self.emit("camera-state", p);
    }

    pub fn modes(&self, p: ModesPayload) {
        self.snap.lock().unwrap().modes = p.clone();
        self.emit("modes-changed", p);
    }

    pub fn controls(&self, controls: Vec<Control>) {
        self.snap.lock().unwrap().controls = controls.clone();
        #[derive(Clone, Serialize)]
        struct P {
            controls: Vec<Control>,
        }
        self.emit("controls-changed", P { controls });
    }

    pub fn prefs(&self, cfg: &Config) {
        self.snap.lock().unwrap().prefs = Prefs::from(cfg);
    }

    pub fn recording(&self, p: RecordingPayload) {
        self.snap.lock().unwrap().recording = Some(p.clone());
        self.emit("recording", p);
    }

    pub fn capture_saved(&self, path: &Path, kind: CaptureKind, interrupted: bool) {
        self.emit(
            "capture-saved",
            CaptureSaved {
                path: path.to_owned(),
                kind,
                interrupted,
            },
        );
    }

    pub fn stats(&self, s: Stats) {
        self.emit("stats", s);
    }

    pub fn error(&self, message: impl Into<String>) {
        #[derive(Clone, Serialize)]
        struct P {
            message: String,
        }
        let message = message.into();
        tracing::warn!("ui error: {message}");
        self.emit("error", P { message });
    }
}
