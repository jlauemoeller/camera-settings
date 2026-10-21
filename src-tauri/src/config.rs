//! App preferences and saved control values, in `$XDG_CONFIG_HOME/camera-settings/config.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use camera_control::Mode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VideoQuality {
    #[default]
    Original,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureKind {
    #[default]
    Photo,
    Video,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraPrefs {
    /// Control id → value the user chose. Absent means "device default".
    pub controls: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `None` = XDG default.
    pub photo_dir: Option<PathBuf>,
    pub video_dir: Option<PathBuf>,
    pub mode: Option<Mode>,
    pub video_quality: VideoQuality,
    pub audio: bool,
    pub capture_kind: CaptureKind,
    /// Keyed by camera serial.
    pub cameras: BTreeMap<String, CameraPrefs>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            photo_dir: None,
            video_dir: None,
            mode: None,
            video_quality: VideoQuality::Original,
            audio: true,
            capture_kind: CaptureKind::Photo,
            cameras: BTreeMap::new(),
        }
    }
}

pub fn default_photo_dir() -> PathBuf {
    dirs::picture_dir().unwrap_or_else(|| home().join("Pictures"))
}

pub fn default_video_dir() -> PathBuf {
    dirs::video_dir().unwrap_or_else(|| home().join("Videos"))
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
}

impl Config {
    pub fn photo_dir(&self) -> PathBuf {
        self.photo_dir.clone().unwrap_or_else(default_photo_dir)
    }

    pub fn video_dir(&self) -> PathBuf {
        self.video_dir.clone().unwrap_or_else(default_video_dir)
    }

    pub fn camera(&mut self, serial: &str) -> &mut CameraPrefs {
        self.cameras.entry(serial.to_owned()).or_default()
    }
}

/// Thread-safe config with a dirty flag so frequent control writes are saved in batches.
pub struct ConfigStore {
    path: PathBuf,
    cfg: Mutex<Config>,
    dirty: AtomicBool,
    /// Serialises writers: they share the temp file.
    save_lock: Mutex<()>,
}

impl ConfigStore {
    pub fn load() -> Self {
        let path = dirs::config_dir()
            .unwrap_or_else(|| home().join(".config"))
            .join("camera-settings/config.toml");
        Self::load_from(path)
    }

    pub fn load_from(path: PathBuf) -> Self {
        let cfg = match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("ignoring unreadable config {}: {e}", path.display());
                Config::default()
            }),
            Err(_) => Config::default(),
        };
        Self {
            path,
            cfg: Mutex::new(cfg),
            dirty: AtomicBool::new(false),
            save_lock: Mutex::new(()),
        }
    }

    pub fn get(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }

    /// Mutate; the change is written by the next [`save_if_dirty`](Self::save_if_dirty).
    pub fn update<R>(&self, f: impl FnOnce(&mut Config) -> R) -> R {
        let r = f(&mut self.cfg.lock().unwrap());
        self.dirty.store(true, Ordering::Release);
        r
    }

    pub fn save_if_dirty(&self) {
        let _guard = self.save_lock.lock().unwrap();
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return;
        }
        let text = match toml::to_string_pretty(&*self.cfg.lock().unwrap()) {
            Ok(t) => t,
            Err(e) => return tracing::error!("serialising config: {e}"),
        };
        if let Err(e) = write_atomic(&self.path, text.as_bytes()) {
            tracing::error!("saving {}: {e}", self.path.display());
        }
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let dir = std::env::temp_dir().join(format!("camera-settings-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        let store = ConfigStore::load_from(path.clone());
        assert_eq!(store.get().video_quality, VideoQuality::Original);
        store.update(|c| {
            c.mode = Some(Mode {
                width: 3840,
                height: 2160,
                fps_n: 30,
                fps_d: 1,
            });
            c.camera("ABC").controls.insert("zoom".into(), 200);
        });
        store.save_if_dirty();
        let again = ConfigStore::load_from(path).get();
        assert_eq!(again.mode.unwrap().width, 3840);
        assert_eq!(again.cameras["ABC"].controls["zoom"], 200);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tolerates_partial_files() {
        let cfg: Config = toml::from_str("audio = false").unwrap();
        assert!(!cfg.audio);
        assert_eq!(cfg.capture_kind, CaptureKind::Photo);
    }
}
