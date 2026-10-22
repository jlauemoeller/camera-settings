//! Controls worker: owns the camera's control fd on its own thread, so a slow or vanished device
//! can only ever delay this thread, never the UI or the session.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use camera_control::{CameraInfo, Control, Device, Error, Mode, allowlist};
use serde::Serialize;

use crate::config::ConfigStore;
use crate::ui::Ui;

pub type Reply<T> = Sender<Result<T, String>>;

pub enum Msg {
    /// Open the device, restore saved values, enumerate modes. If this camera is already open,
    /// just return its modes: saved values are applied once per connection, not on every retry
    /// (another app may be streaming and the user adjusting live).
    Open {
        info: CameraInfo,
        reply: Reply<Vec<Mode>>,
    },
    Close,
    Set {
        id: String,
        value: i64,
        reply: Reply<()>,
    },
    Reset {
        id: String,
        reply: Reply<()>,
    },
    ResetAll {
        reply: Reply<()>,
    },
    /// The stream's current frame size (the other app's, while it owns the stream).
    CurrentSize {
        reply: Reply<(u32, u32)>,
    },
    /// Settle a [`SettingDiff`] conflict: adopt the camera's values, or apply the app's.
    Resolve {
        adopt: bool,
        reply: Reply<()>,
    },
}

/// One setting where the camera (in use by another app) differs from the app's settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingDiff {
    pub id: &'static str,
    pub label: &'static str,
    /// `None`: decided automatically (its auto mode is on), so the value doesn't matter.
    pub camera: Option<i64>,
    pub app: Option<i64>,
}

/// (control, its auto mode, "is this auto-mode value manual?").
type Dependent = (&'static str, &'static str, fn(i64) -> bool);

/// Manual values that only matter while their auto mode is off.
const DEPENDENTS: &[Dependent] = &[
    ("exposure_time", "exposure_auto", |v| v == 1),
    ("focus", "focus_auto", |v| v == 0),
    ("white_balance_temperature", "white_balance_auto", |v| v == 0),
];

/// Compare the camera's current controls with the app's settings (saved value, else the
/// device default, which is what the camera has after power-up). A manual value whose auto
/// mode is on counts as "automatic" on that side, so e.g. the exposure the camera's
/// auto-exposure picked isn't reported as a difference.
pub fn diff(controls: &[Control], saved: &BTreeMap<String, i64>) -> Vec<SettingDiff> {
    let app = |c: &Control| saved.get(c.id).copied().unwrap_or(c.default);
    let by_id = |id: &str| controls.iter().find(|c| c.id == id);
    let mut out: Vec<SettingDiff> = controls
        .iter()
        .filter(|c| !c.read_only)
        .filter_map(|c| {
            let (camera, app_value) = match DEPENDENTS.iter().find(|(dep, ..)| *dep == c.id) {
                Some((_, master, is_manual)) => {
                    let m = by_id(master)?;
                    (
                        is_manual(m.value).then_some(c.value),
                        is_manual(app(m)).then_some(app(c)),
                    )
                }
                None => (Some(c.value), Some(app(c))),
            };
            (camera != app_value).then_some(SettingDiff {
                id: c.id,
                label: c.label,
                camera,
                app: app_value,
            })
        })
        .collect();
    out.sort_by_key(|d| allowlist::restore_order(d.id));
    out
}

#[derive(Clone)]
pub struct Controls {
    tx: Sender<Msg>,
}

/// Error string the frontend treats as "ignore": the camera is gone and the UI already shows it.
pub const DISCONNECTED: &str = "disconnected";

impl Controls {
    pub fn spawn(ui: Arc<Ui>, config: Arc<ConfigStore>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("controls".into())
            .spawn(move || {
                Worker {
                    ui,
                    config,
                    dev: None,
                    conflict: false,
                }
                .run(rx)
            })
            .expect("spawn controls thread");
        Self { tx }
    }

    pub fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
    }

    /// Send a request and wait at most `timeout` for the reply.
    pub fn call<T>(&self, timeout: Duration, make: impl FnOnce(Reply<T>) -> Msg) -> Result<T, String> {
        let (reply, rx) = mpsc::channel();
        self.send(make(reply));
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => Err("camera did not respond".into()),
        }
    }
}

struct Open {
    dev: Device,
    serial: String,
    node: std::path::PathBuf,
    modes: Vec<Mode>,
}

struct Worker {
    ui: Arc<Ui>,
    config: Arc<ConfigStore>,
    dev: Option<Open>,
    /// Opened while another app streams, with settings that differ: nothing has been applied
    /// yet and the user is asked which side wins.
    conflict: bool,
}

impl Worker {
    fn run(mut self, rx: Receiver<Msg>) {
        loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            // Batches the many writes a dragged slider produces into one save.
            self.config.save_if_dirty();
        }
        self.config.save_if_dirty();
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Open { info, reply } => {
                let _ = reply.send(self.open(&info));
            }
            Msg::Close => {
                self.dev = None;
                self.set_conflict(None);
            }
            Msg::Set { id, value, reply } => {
                let r = self.write(&id, Some(value));
                let _ = reply.send(r);
            }
            Msg::Reset { id, reply } => {
                let r = self.write(&id, None);
                let _ = reply.send(r);
            }
            Msg::ResetAll { reply } => {
                let _ = reply.send(self.reset_all());
            }
            Msg::CurrentSize { reply } => {
                let r = match &self.dev {
                    Some(open) => open.dev.current_size().map_err(|e| e.to_string()),
                    None => Err(DISCONNECTED.into()),
                };
                let _ = reply.send(r);
            }
            Msg::Resolve { adopt, reply } => {
                let _ = reply.send(self.resolve(adopt));
            }
        }
    }

    fn open(&mut self, info: &CameraInfo) -> Result<Vec<Mode>, String> {
        if let Some(open) = &self.dev
            && open.node == info.node
            && open.serial == info.serial
            && info.is_present()
        {
            return Ok(open.modes.clone());
        }
        self.dev = None;
        self.set_conflict(None);
        let dev = Device::open(info).map_err(|e| e.to_string())?;
        let modes = dev.modes().map_err(|e| e.to_string())?;
        let saved = self.saved(&info.serial);

        // Another app is using the camera: don't silently change its picture. If the settings
        // differ, leave the camera as it is and ask.
        if dev.stream_in_use().unwrap_or(false) {
            let differences = diff(&dev.controls().map_err(|e| e.to_string())?, &saved);
            if !differences.is_empty() {
                tracing::info!(
                    "camera in use with differing settings; asking: {:?}",
                    differences.iter().map(|d| (d.id, d.camera, d.app)).collect::<Vec<_>>()
                );
                self.dev = Some(Open {
                    dev,
                    serial: info.serial.clone(),
                    node: info.node.clone(),
                    modes: modes.clone(),
                });
                self.publish();
                self.set_conflict(Some(differences));
                return Ok(modes);
            }
        }

        let mut saved: Vec<_> = saved.into_iter().collect();
        saved.sort_by_key(|(id, _)| allowlist::restore_order(id));
        tracing::info!("camera opened; applying {} saved setting(s)", saved.len());
        for (id, value) in saved {
            // The camera forgets everything on power loss; failures (e.g. a manual value whose
            // auto mode is on) are expected and harmless.
            if let Err(e) = dev.set(&id, value) {
                if e.is_gone() {
                    return Err(DISCONNECTED.into());
                }
                tracing::debug!("restore {id}={value}: {e}");
            }
        }
        self.dev = Some(Open {
            dev,
            serial: info.serial.clone(),
            node: info.node.clone(),
            modes: modes.clone(),
        });
        self.publish();
        Ok(modes)
    }

    fn saved(&self, serial: &str) -> BTreeMap<String, i64> {
        self.config
            .get()
            .cameras
            .get(serial)
            .map(|c| c.controls.clone())
            .unwrap_or_default()
    }

    fn set_conflict(&mut self, differences: Option<Vec<SettingDiff>>) {
        if differences.is_none() && !self.conflict {
            return;
        }
        self.conflict = differences.is_some();
        self.ui.settings_conflict(differences);
    }

    /// Recomputes the differences now (the user may have changed things meanwhile), then either
    /// saves the camera's values as the app's settings or writes the app's values to the camera.
    fn resolve(&mut self, adopt: bool) -> Result<(), String> {
        let Some(open) = &self.dev else {
            return Err(DISCONNECTED.into());
        };
        let current = open.dev.controls().map_err(|e| e.to_string())?;
        let differences = diff(&current, &self.saved(&open.serial));
        if adopt {
            let serial = open.serial.clone();
            self.config.update(|c| {
                let saved = &mut c.camera(&serial).controls;
                for d in &differences {
                    // `None`: automatic on the camera; its auto-mode row carries the change.
                    if let Some(v) = d.camera {
                        saved.insert(d.id.to_owned(), v);
                    }
                }
            });
        } else {
            for d in &differences {
                let Some(v) = d.app else { continue };
                if let Err(e) = open.dev.set(d.id, v) {
                    if e.is_gone() {
                        return self.fail(e);
                    }
                    tracing::debug!("apply {}={v}: {e}", d.id);
                }
            }
        }
        self.set_conflict(None);
        self.publish();
        Ok(())
    }

    /// `value: None` = reset to the device default.
    fn write(&mut self, id: &str, value: Option<i64>) -> Result<(), String> {
        let Some(open) = &self.dev else {
            return Err(DISCONNECTED.into());
        };
        let result = (|| {
            let value = match value {
                Some(v) => v,
                None => open.dev.control(id)?.default,
            };
            open.dev.set(id, value)
        })();
        match result {
            Ok(()) => {
                let serial = open.serial.clone();
                self.config.update(|c| {
                    let saved = &mut c.camera(&serial).controls;
                    match value {
                        Some(v) => saved.insert(id.to_owned(), v),
                        None => saved.remove(id),
                    };
                });
                self.publish();
                Ok(())
            }
            Err(e) => self.fail(e),
        }
    }

    fn reset_all(&mut self) -> Result<(), String> {
        let Some(open) = &self.dev else {
            return Err(DISCONNECTED.into());
        };
        let mut controls = open.dev.controls().map_err(|e| e.to_string())?;
        controls.sort_by_key(|c| allowlist::restore_order(c.id));
        for c in controls.iter().filter(|c| !c.read_only) {
            if let Err(e) = open.dev.set(c.id, c.default) {
                if e.is_gone() {
                    return self.fail(e);
                }
                tracing::debug!("reset {}: {e}", c.id);
            }
        }
        let serial = open.serial.clone();
        self.config.update(|c| c.camera(&serial).controls.clear());
        self.publish();
        Ok(())
    }

    /// Re-query every control (INACTIVE flags change with auto modes) and send them to the UI.
    fn publish(&mut self) {
        let Some(open) = &self.dev else { return };
        match open.dev.controls() {
            Ok(c) => self.ui.controls(c),
            Err(e) if e.is_gone() => self.dev = None,
            Err(e) => tracing::warn!("query controls: {e}"),
        }
    }

    fn fail(&mut self, e: Error) -> Result<(), String> {
        if e.is_gone() {
            // SPEC §6.3.5: fail fast and drop silently; the session handles the disconnect.
            self.dev = None;
            return Err(DISCONNECTED.into());
        }
        Err(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camera_control::{Kind, allowlist::Group};

    fn c(id: &'static str, value: i64, default: i64) -> Control {
        Control {
            id,
            label: id,
            group: Group::Color,
            kind: Kind::Int,
            min: 0,
            max: 10_000,
            step: 1,
            default,
            value,
            options: None,
            unit: None,
            inactive: false,
            read_only: false,
        }
    }

    fn saved(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn plain_differences_against_saved_or_default() {
        let cam = [c("zoom", 100, 100), c("fov", 0, 0), c("brightness", 140, 128)];
        // zoom saved at 250 → differs; fov saved 0 = camera; brightness unsaved → default 128 ≠ 140.
        let d = diff(&cam, &saved(&[("zoom", 250), ("fov", 0)]));
        let ids: Vec<_> = d.iter().map(|d| (d.id, d.camera, d.app)).collect();
        assert_eq!(
            ids,
            [("zoom", Some(100), Some(250)), ("brightness", Some(140), Some(128))]
        );
    }

    #[test]
    fn identical_settings_mean_no_question() {
        let cam = [
            c("zoom", 250, 100),
            c("white_balance_auto", 1, 1),
            c("white_balance_temperature", 4630, 4000),
        ];
        // Temperature is picked by auto white balance on both sides: not a difference.
        assert!(diff(&cam, &saved(&[("zoom", 250)])).is_empty());
    }

    #[test]
    fn manual_values_only_compared_when_manual() {
        // Camera: manual WB at 3000 K. App: auto WB (default) → the auto row differs and the
        // temperature shows camera 3000 vs app "automatic".
        let cam = [
            c("white_balance_auto", 0, 1),
            c("white_balance_temperature", 3000, 4000),
        ];
        let d = diff(&cam, &saved(&[]));
        assert_eq!(d.len(), 2);
        assert_eq!(
            (d[0].id, d[0].camera, d[0].app),
            ("white_balance_auto", Some(0), Some(1))
        );
        assert_eq!(
            (d[1].id, d[1].camera, d[1].app),
            ("white_balance_temperature", Some(3000), None)
        );

        // Both manual, same temperature → nothing; different → temperature only.
        let s = saved(&[("white_balance_auto", 0), ("white_balance_temperature", 3000)]);
        assert!(diff(&cam, &s).is_empty());
        let s = saved(&[("white_balance_auto", 0), ("white_balance_temperature", 5000)]);
        let d = diff(&cam, &s);
        assert_eq!((d.len(), d[0].camera, d[0].app), (1, Some(3000), Some(5000)));
    }

    #[test]
    fn auto_modes_sort_before_their_values() {
        let cam = [c("exposure_time", 300, 250), c("exposure_auto", 1, 3)];
        let d = diff(&cam, &saved(&[]));
        assert_eq!(d[0].id, "exposure_auto");
        assert_eq!((d[1].id, d[1].camera, d[1].app), ("exposure_time", Some(300), None));
    }
}
