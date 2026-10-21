//! udev monitor on a dedicated thread. It only signals *that* something relevant changed; the
//! consumer re-scans sysfs ([`crate::discovery::find_camera`]) to learn what is there now.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotplugEvent {
    Added,
    Removed,
    Changed,
}

pub struct HotplugMonitor {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HotplugMonitor {
    /// Watch `video4linux` and `usb` add/remove events. `on_event` runs on the monitor thread.
    pub fn spawn(on_event: impl Fn(HotplugEvent) + Send + 'static) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        // libudev handles aren't `Send`: create the socket on the thread that uses it.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new().name("udev-monitor".into()).spawn(move || {
            let socket = udev::MonitorBuilder::new()
                .and_then(|b| b.match_subsystem("video4linux"))
                .and_then(|b| b.match_subsystem("usb"))
                .and_then(|b| b.listen());
            match socket {
                Ok(socket) => {
                    let _ = ready_tx.send(Ok(()));
                    run(socket, &stop2, on_event);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        })?;
        ready_rx.recv().map_err(std::io::Error::other)??;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

fn run(socket: udev::MonitorSocket, stop: &AtomicBool, on_event: impl Fn(HotplugEvent)) {
    while !stop.load(Ordering::Relaxed) {
        // SAFETY: the socket outlives this borrowed fd (both live for the loop iteration).
        let fd = unsafe { BorrowedFd::borrow_raw(socket.as_fd().as_raw_fd()) };
        let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
        match poll(&mut fds, PollTimeout::from(500u16)) {
            Ok(0) => continue,
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => {
                tracing::error!("udev poll failed: {e}");
                return;
            }
        }
        for event in socket.iter() {
            if !is_relevant(&event) {
                continue;
            }
            let kind = match event.event_type() {
                udev::EventType::Add | udev::EventType::Bind => HotplugEvent::Added,
                udev::EventType::Remove | udev::EventType::Unbind => HotplugEvent::Removed,
                _ => HotplugEvent::Changed,
            };
            tracing::debug!(?kind, path = ?event.syspath(), "hotplug");
            on_event(kind);
        }
    }
}

fn is_relevant(event: &udev::Event) -> bool {
    match event.subsystem().and_then(|s| s.to_str()) {
        Some("video4linux") => true,
        // Only whole USB devices, not every interface; VID:PID isn't available on remove for
        // all kernels, so any usb_device add/remove triggers a cheap rescan.
        Some("usb") => event.devtype().and_then(|t| t.to_str()) == Some("usb_device"),
        _ => false,
    }
}

impl Drop for HotplugMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
