# Camera Settings

Linux desktop app (Tauri 2 + Rust + Svelte 5) for the Logitech BRIO: live preview, the safe camera
controls, field of view, stream modes, and test captures. It keeps working when the camera is unplugged at any moment.
See [SPEC.md](SPEC.md) for the specification and [research/FINDINGS.md](research/FINDINGS.md) for what the device exposes.

## Build

```sh
sudo dnf install gstreamer1-devel gstreamer1-plugins-base-devel webkit2gtk4.1-devel gtk3-devel \
  libsoup3-devel systemd-devel openssl-devel librsvg2-devel
npm install
npx tauri dev                           # run in development
npx tauri build --bundles rpm,deb       # target/release/bundle/{rpm,deb}/
```

Runtime needs GStreamer base/good/bad-free plus `pipewire-gstreamer` (the packages declare these).

Run with `camera-settings --debug` to enable the web inspector and the webview's right-click menu.

## Layout

| Path | What |
|---|---|
| `crates/camera-control/` | Library: sysfs discovery, udev hotplug, allowlisted V4L2 + UVC XU (FOV only) controls. No Tauri, no GStreamer. |
| `src-tauri/` | App crate: session state machine, GStreamer pipeline, config, Tauri commands. |
| `src/` | Svelte frontend. |

## Tests

```sh
cargo test --workspace                                  # unit tests, no camera needed
cargo test -p camera-control -- --ignored               # against an attached BRIO; writes allowlisted controls and restores them
npm run check
```

### Driving a debug build headlessly

Debug builds read commands from stdin when `CAMERA_SETTINGS_DEBUG_STDIN=1` is set (with `npx vite` running):

```sh
tail -f cmds.txt | CAMERA_SETTINGS_DEBUG_STDIN=1 RUST_LOG=camera_settings=debug ./target/debug/camera-settings
echo "rec audio" >> cmds.txt   # still | rec [audio] [compact] | stop | mode 3840x2160@30 | set <id> <v> | reset <id>
                               # pause | resume | hidden 0|1 | minimized 0|1 | ui drawer|info|show|scroll N|quit
```

`CAMERA_SETTINGS_DEBUG_AUDIO_NODE=<node>` (debug builds) forces the recording mic, e.g. a bogus name to
exercise the mic-failure path.

### Hot-unplug acceptance tests (SPEC §6.4)

Simulate an unplug without touching the cable (`4-1` is the BRIO's USB path; see `/sys/bus/usb/devices/*/product`):

```sh
echo 0 | sudo tee /sys/bus/usb/devices/4-1/authorized   # unplug
echo 1 | sudo tee /sys/bus/usb/devices/4-1/authorized   # replug
for i in $(seq 20); do echo 0 | sudo tee /sys/bus/usb/devices/4-1/authorized; sleep 2; \
  echo 1 | sudo tee /sys/bus/usb/devices/4-1/authorized; sleep 4; done          # H9
fuser /dev/video*                                                                # H10: empty while minimised
```
