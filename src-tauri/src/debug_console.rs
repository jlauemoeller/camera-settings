//! Debug builds only: drive the backend from stdin, for headless testing of the session.
//! Enabled with `CAMERA_SETTINGS_DEBUG_STDIN=1`. One command per line:
//!
//! ```text
//! still | rec [audio] [compact] | stop | mode 3840x2160@30 | set <id> <value> | reset <id>
//! pause | resume | hidden <0|1> | minimized <0|1> | ui <drawer|info|show|scroll N|quit>
//! ```

use std::io::BufRead;
use std::time::Duration;

use camera_control::Mode;
use tauri::{AppHandle, Emitter};

use crate::config::VideoQuality;
use crate::controls::{self, Controls};
use crate::session::{Msg, SessionHandle};

pub fn spawn(app: AppHandle, session: SessionHandle, controls: Controls) {
    if std::env::var_os("CAMERA_SETTINGS_DEBUG_STDIN").is_none() {
        return;
    }
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let words: Vec<&str> = line.split_whitespace().collect();
            let t = Duration::from_secs(3);
            let result = match words.as_slice() {
                ["still"] => session.call(t, Msg::CaptureStill),
                ["rec", flags @ ..] => session.call(t, |reply| Msg::StartRecording {
                    audio: flags.contains(&"audio"),
                    quality: if flags.contains(&"compact") {
                        VideoQuality::Compact
                    } else {
                        VideoQuality::Original
                    },
                    reply,
                }),
                ["stop"] => session.call(t, Msg::StopRecording),
                ["mode", spec] => match parse_mode(spec) {
                    Some(m) => session.call(t, |r| Msg::SetMode(m, r)),
                    None => Err("mode WxH@fps".into()),
                },
                ["set", id, value] => match value.parse() {
                    Ok(value) => controls.call(t, |reply| controls::Msg::Set {
                        id: id.to_string(),
                        value,
                        reply,
                    }),
                    Err(_) => Err("bad value".into()),
                },
                ["reset", id] => controls.call(t, |reply| controls::Msg::Reset {
                    id: id.to_string(),
                    reply,
                }),
                ["pause"] => {
                    session.send(Msg::ManualPause(true));
                    Ok(())
                }
                ["resume"] => {
                    session.send(Msg::ManualPause(false));
                    Ok(())
                }
                ["hidden", v] => {
                    session.send(Msg::Hidden(*v == "1"));
                    Ok(())
                }
                ["minimized", v] => {
                    session.send(Msg::Minimized(*v == "1"));
                    Ok(())
                }
                ["ui", rest @ ..] => app.emit("debug-ui", rest.join(" ")).map_err(|e| e.to_string()),
                _ => Err(format!("unknown command: {line}")),
            };
            tracing::info!("debug> {line} → {result:?}");
        }
    });
}

fn parse_mode(spec: &str) -> Option<Mode> {
    let (size, fps) = spec.split_once('@')?;
    let (w, h) = size.split_once('x')?;
    Some(Mode {
        width: w.parse().ok()?,
        height: h.parse().ok()?,
        fps_n: fps.parse().ok()?,
        fps_d: 1,
    })
}
