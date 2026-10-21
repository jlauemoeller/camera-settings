//! Preview transport (SPEC §8.2): JPEG frames to the webview over an `ipc::Channel` as raw bytes.
//!
//! Flow control: at most [`MAX_IN_FLIGHT`] frames are outstanding until the frontend acks one,
//! so a slow webview drops frames here instead of piling them up in Tauri's channel queue.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use tauri::ipc::{Channel, InvokeResponseBody};

const MAX_IN_FLIGHT: u32 = 2;
/// Assume acks were lost (e.g. the page reloaded) after this long.
const ACK_TIMEOUT_MS: u64 = 1000;

pub struct Preview {
    channel: Mutex<Option<Channel<InvokeResponseBody>>>,
    in_flight: AtomicU32,
    last_ack_ms: AtomicU64,
    epoch: Instant,
    pub sent: AtomicU64,
    pub skipped: AtomicU64,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            channel: Mutex::new(None),
            in_flight: AtomicU32::new(0),
            last_ack_ms: AtomicU64::new(0),
            epoch: Instant::now(),
            sent: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
        }
    }
}

impl Preview {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    pub fn set_channel(&self, ch: Channel<InvokeResponseBody>) {
        *self.channel.lock().unwrap() = Some(ch);
        self.in_flight.store(0, Ordering::Relaxed);
        self.last_ack_ms.store(self.now_ms(), Ordering::Relaxed);
    }

    /// Called on the GStreamer streaming thread for every preview frame.
    pub fn offer(&self, frame: Vec<u8>) {
        let now = self.now_ms();
        if self.in_flight.load(Ordering::Relaxed) >= MAX_IN_FLIGHT {
            if now.saturating_sub(self.last_ack_ms.load(Ordering::Relaxed)) < ACK_TIMEOUT_MS {
                self.skipped.fetch_add(1, Ordering::Relaxed);
                return;
            }
            self.in_flight.store(0, Ordering::Relaxed);
        }
        let Some(ch) = self.channel.lock().unwrap().clone() else {
            return;
        };
        if ch.send(InvokeResponseBody::Raw(frame)).is_ok() {
            self.in_flight.fetch_add(1, Ordering::Relaxed);
            self.sent.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn ack(&self) {
        let _ = self
            .in_flight
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| Some(v.saturating_sub(1)));
        self.last_ack_ms.store(self.now_ms(), Ordering::Relaxed);
    }
}
