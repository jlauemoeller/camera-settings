//! Find the PipeWire node of the camera's own microphone (SPEC §5.2): the ALSA card that shares
//! the camera's USB device, then the `Audio/Source` node on that card. Never the default source.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;

/// `node.name` of the source node on ALSA card `card`, for `pipewiresrc target-object`.
pub fn find_mic_node(card: u32) -> Option<String> {
    from_pw_dump(card).or_else(|| from_device_monitor(card))
}

fn card_matches(props: &serde_json::Map<String, serde_json::Value>, card: u32) -> bool {
    ["api.alsa.pcm.card", "alsa.card", "api.alsa.card"].iter().any(|k| {
        props
            .get(*k)
            .is_some_and(|v| v.as_u64() == Some(u64::from(card)) || v.as_str() == Some(&card.to_string()))
    })
}

fn from_pw_dump(card: u32) -> Option<String> {
    let mut child = Command::new("pw-dump")
        .arg("--no-colors")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    // pw-dump can hang if PipeWire is wedged; read on a helper thread and give up after 2 s.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let out = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    let _ = child.kill();
    let _ = child.wait();
    let objects: Vec<serde_json::Value> = serde_json::from_slice(&out.ok()?).ok()?;
    objects.iter().find_map(|o| {
        if !o.get("type")?.as_str()?.ends_with(":Node") {
            return None;
        }
        let props = o.get("info")?.get("props")?.as_object()?;
        let class = props.get("media.class")?.as_str()?;
        (class == "Audio/Source" && card_matches(props, card))
            .then(|| props.get("node.name")?.as_str().map(str::to_owned))
            .flatten()
    })
}

fn from_device_monitor(card: u32) -> Option<String> {
    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Audio/Source"), None);
    monitor.start().ok()?;
    let found = monitor.devices().into_iter().find_map(|d| {
        let props = d.properties()?;
        let has_card = ["api.alsa.pcm.card", "alsa.card", "api.alsa.card"].iter().any(|k| {
            props.get::<i32>(*k).ok().map(|v| v as u32) == Some(card)
                || props.get::<String>(*k).ok().as_deref() == Some(&card.to_string())
        });
        has_card.then(|| props.get::<String>("node.name").ok()).flatten()
    });
    monitor.stop();
    found
}
