//! Controls worker: owns the camera's control fd on its own thread, so a slow or vanished device
//! can only ever delay this thread, never the UI or the session.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use camera_control::{CameraInfo, Device, Error, Mode, allowlist};

use crate::config::ConfigStore;
use crate::ui::Ui;

pub type Reply<T> = Sender<Result<T, String>>;

pub enum Msg {
    /// Open the device, restore saved values, enumerate modes.
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
            .spawn(move || Worker { ui, config, dev: None }.run(rx))
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
}

struct Worker {
    ui: Arc<Ui>,
    config: Arc<ConfigStore>,
    dev: Option<Open>,
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
            Msg::Close => self.dev = None,
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
        }
    }

    fn open(&mut self, info: &CameraInfo) -> Result<Vec<Mode>, String> {
        self.dev = None;
        let dev = Device::open(info).map_err(|e| e.to_string())?;
        let modes = dev.modes().map_err(|e| e.to_string())?;
        let saved = self
            .config
            .get()
            .cameras
            .get(&info.serial)
            .map(|c| c.controls.clone())
            .unwrap_or_default();
        let mut saved: Vec<_> = saved.into_iter().collect();
        saved.sort_by_key(|(id, _)| allowlist::restore_order(id));
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
        });
        self.publish();
        Ok(modes)
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
