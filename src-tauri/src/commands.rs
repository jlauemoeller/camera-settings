//! Tauri commands (SPEC §8.3). All are `async` and hand work to backend threads, so the UI
//! thread never does device or file I/O; waits are bounded by timeouts.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use camera_control::Mode;
use serde::Deserialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::config::{CaptureKind, ConfigStore, VideoQuality};
use crate::controls::{self, Controls};
use crate::preview::Preview;
use crate::session::{Msg, SessionHandle};
use crate::ui::{Prefs, Snapshot, Ui};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(1);
const SESSION_TIMEOUT: Duration = Duration::from_secs(3);

pub struct AppState {
    pub ui: Arc<Ui>,
    pub config: Arc<ConfigStore>,
    pub controls: Controls,
    pub preview: Arc<Preview>,
    pub session: SessionHandle,
}

type CmdResult<T = ()> = Result<T, String>;

/// Run blocking work on Tauri's blocking pool.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_snapshot(state: State<'_, AppState>) -> CmdResult<Snapshot> {
    let mut snap = state.ui.snapshot();
    snap.prefs = Prefs::from(&state.config.get());
    Ok(snap)
}

#[tauri::command]
pub async fn open_preview(state: State<'_, AppState>, channel: Channel<InvokeResponseBody>) -> CmdResult {
    state.preview.set_channel(channel);
    Ok(())
}

#[tauri::command]
pub async fn preview_ack(state: State<'_, AppState>) -> CmdResult {
    state.preview.ack();
    Ok(())
}

#[tauri::command]
pub async fn set_mode(state: State<'_, AppState>, mode: Mode) -> CmdResult {
    let session = state.session.clone();
    blocking(move || session.call(SESSION_TIMEOUT, |r| Msg::SetMode(mode, r))).await
}

#[tauri::command]
pub async fn set_control(state: State<'_, AppState>, id: String, value: i64) -> CmdResult {
    let c = state.controls.clone();
    blocking(move || c.call(CONTROL_TIMEOUT, |reply| controls::Msg::Set { id, value, reply })).await
}

#[tauri::command]
pub async fn reset_control(state: State<'_, AppState>, id: String) -> CmdResult {
    let c = state.controls.clone();
    blocking(move || c.call(CONTROL_TIMEOUT, |reply| controls::Msg::Reset { id, reply })).await
}

#[tauri::command]
pub async fn reset_all(state: State<'_, AppState>) -> CmdResult {
    let c = state.controls.clone();
    blocking(move || c.call(Duration::from_secs(3), |reply| controls::Msg::ResetAll { reply })).await
}

#[tauri::command]
pub async fn capture_still(state: State<'_, AppState>) -> CmdResult {
    let session = state.session.clone();
    blocking(move || session.call(SESSION_TIMEOUT, Msg::CaptureStill)).await
}

#[derive(Deserialize)]
pub struct RecordArgs {
    audio: bool,
    quality: VideoQuality,
}

#[tauri::command]
pub async fn start_recording(state: State<'_, AppState>, options: RecordArgs) -> CmdResult {
    let session = state.session.clone();
    blocking(move || {
        session.call(SESSION_TIMEOUT, |reply| Msg::StartRecording {
            audio: options.audio,
            quality: options.quality,
            reply,
        })
    })
    .await
}

#[tauri::command]
pub async fn stop_recording(state: State<'_, AppState>) -> CmdResult {
    let session = state.session.clone();
    blocking(move || session.call(SESSION_TIMEOUT, Msg::StopRecording)).await
}

/// Partial preferences. For folders, an empty string resets to the XDG default.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PrefsPatch {
    photo_dir: Option<String>,
    video_dir: Option<String>,
    video_quality: Option<VideoQuality>,
    audio: Option<bool>,
    capture_kind: Option<CaptureKind>,
}

fn folder(value: String) -> CmdResult<Option<PathBuf>> {
    if value.is_empty() {
        return Ok(None);
    }
    let p = PathBuf::from(value);
    if !p.is_absolute() {
        return Err("folder must be an absolute path".into());
    }
    Ok(Some(p))
}

#[tauri::command]
pub async fn set_prefs(state: State<'_, AppState>, prefs: PrefsPatch) -> CmdResult<Prefs> {
    let config = state.config.clone();
    let ui = state.ui.clone();
    blocking(move || {
        let photo = prefs.photo_dir.map(folder).transpose()?;
        let video = prefs.video_dir.map(folder).transpose()?;
        let cfg = config.update(|c| {
            if let Some(p) = photo {
                c.photo_dir = p;
            }
            if let Some(v) = video {
                c.video_dir = v;
            }
            if let Some(q) = prefs.video_quality {
                c.video_quality = q;
            }
            if let Some(a) = prefs.audio {
                c.audio = a;
            }
            if let Some(k) = prefs.capture_kind {
                c.capture_kind = k;
            }
            c.clone()
        });
        config.save_if_dirty();
        ui.prefs(&cfg);
        Ok(Prefs::from(&cfg))
    })
    .await
}

/// Only files inside the configured capture folders may be opened or revealed.
fn check_capture_path(config: &ConfigStore, path: &Path) -> CmdResult<PathBuf> {
    let cfg = config.get();
    let path = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let allowed = [cfg.photo_dir(), cfg.video_dir()]
        .into_iter()
        .filter_map(|d| std::fs::canonicalize(d).ok())
        .any(|d| path.starts_with(d));
    if allowed {
        Ok(path)
    } else {
        Err("not a capture file".into())
    }
}

#[tauri::command]
pub async fn reveal(app: AppHandle, state: State<'_, AppState>, path: PathBuf) -> CmdResult {
    let config = state.config.clone();
    blocking(move || {
        let path = check_capture_path(&config, &path)?;
        app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn open_capture(app: AppHandle, state: State<'_, AppState>, path: PathBuf) -> CmdResult {
    let config = state.config.clone();
    blocking(move || {
        let path = check_capture_path(&config, &path)?;
        app.opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())
    })
    .await
}

/// Page visibility from the webview's `visibilitychange`.
#[tauri::command]
pub async fn set_hidden(state: State<'_, AppState>, hidden: bool) -> CmdResult {
    state.session.send(Msg::Hidden(hidden));
    Ok(())
}

/// Manual pause/resume (SPEC §6.5 fallback).
#[tauri::command]
pub async fn set_paused(state: State<'_, AppState>, paused: bool) -> CmdResult {
    state.session.send(Msg::ManualPause(paused));
    Ok(())
}

/// Quit button / Ctrl+Q / Ctrl+W: some compositors (e.g. niri) show no window decorations.
/// Exiting runs the shutdown path, which finalizes a running recording first.
#[tauri::command]
pub async fn quit(app: AppHandle) -> CmdResult {
    app.exit(0);
    Ok(())
}
