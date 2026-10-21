# Camera Settings: specification

Working title: **Camera Settings** (package/binary `camera-settings`). The final product name is undecided (see Open questions).
A small Linux desktop app (Tauri 2 + Rust) for the Logitech BRIO: live preview, safe camera settings, and test captures (photo, video with or without the camera's microphone). It must survive the camera being unplugged at any moment.

Background research: [research/FINDINGS.md](research/FINDINGS.md) (what the device exposes) and `research/probe.py` (read-only scanner). "FINDINGS §n" below refers to that file.

---

## 1. Goals

1. Live preview of the camera, letterboxed, full-bleed, like GNOME Camera.
2. Change every **safe** setting (§4): the standard UVC controls, Field of View, and stream mode (resolution/fps).
3. Capture a **still** (the camera's own JPEG, unmodified) and **record video** with or without audio from the BRIO's mic.
4. Save captures to user-selectable folders, defaulting to the XDG Pictures or XDG Videos directory depending on capture media kind.
5. **Never hang**, whatever the camera does. Unplugging shows a camera-off screen and replugging resumes automatically.
6. Minimal, modern, dark UI. All icons from Lucide.

## 2. Non-goals (v1)

- **The privacy LED.** The app never reads or writes the Logitech Peripheral LED control (`ffe52d21-…`, selector 9), even if the kernel parameter `uvcvideo.allow_privacy_override=1` is set. The LED stays under firmware control.
- Writing **any** extension-unit control except FOV. Units `23e49ed0-…` (6), `1f5d4ca9-…` (9) and `2c49d16a-…` (14) look like firmware/debug channels and are never touched. Unidentified selectors of `49e40215-…` (possibly HDR or RightLight) stay out of scope until a USB capture from Logi Tune identifies them (FINDINGS §5).
- Multiple cameras / device picker. Other UVC cameras may work for standard controls but are untested.
- Raw (YUYV/NV12) stream modes. v1 uses MJPEG modes only (§4.3).
- Non-Linux platforms.
- Flatpak/Snap packaging and publishing on crates.io (see §3.1 for why).

## 3. Platform and prerequisites

- Linux only; developed on Fedora 44, kernel 7.2, PipeWire, AMD Radeon 890M (VA-API).
- Runtime: GStreamer 1.x with base/good/bad plugins. Verified present on the dev machine: `v4l2src`, `jpegdec`, `vajpegdec`, `videoscale`, `jpegenc`, `matroskamux`, `pipewiresrc`, `opusenc`, `vaav1enc`, `svtav1enc`, `appsink`.
  **Not present:** `x264enc` and VA-API H.264/H.265 encoders. Don't rely on H.264.
- Build deps still missing on the dev machine: `sudo dnf install gstreamer1-devel gstreamer1-plugins-base-devel webkit2gtk4.1-devel gtk3-devel libsoup3-devel systemd-devel openssl-devel`
- Toolchain available: Rust 1.99, Node 22. Frontend: Svelte 5 + TypeScript + Vite; icons `@lucide/svelte`.
- Device access via logind `uaccess` ACLs (already works for the user; no root, no udev rules needed).

### 3.1 Distribution

- **v1:** build from source (`cargo tauri build`) and native packages from the Tauri bundler: `.rpm` (primary,
  Fedora) and `.deb`. Both depend on the system's GStreamer and WebKitGTK and don't bundle them.
- **Not AppImage:** it would have to bundle WebKitGTK and GStreamer plus plugins, which is large and fragile.
- **Not Flatpak/Snap (v1):** the app needs raw V4L2 ioctls (including UVC extension-unit queries), USB descriptors from
  sysfs and udev hotplug events. Sandboxed apps are meant to use the PipeWire camera portal, which gives
  frames but no control access. `--device=all` works around device access, but udev events aren't reliably
  available inside the sandbox. Revisit once v1 is stable.
- **Not crates.io (app):** `cargo install` is a poor install path for a GUI app. The published crate would have to embed the
  prebuilt frontend (a Node build step), still needs the system `-devel` packages, and installs no desktop entry or
  icon. All crates set `publish = false`. The name `camera-settings` was free on crates.io as of 2026-10-21.
- **Possible later:** publish the device-control layer (discovery + allowlisted V4L2/UVC controls, no Tauri
  dependency) as a library crate if it proves reusable. The workspace layout in §8.1 keeps that option open.

## 4. Settings: the allowlist

The **backend** enforces this list: any control id not in it is rejected before reaching an ioctl. The frontend renders whatever the backend reports (ranges are read from the device at connect time and are not hardcoded; the values below are what this BRIO reports).

### 4.1 Standard V4L2 controls

| Group    | Control                   | V4L2 CID     | Range (step)                        | Default | UI                                                 |
| -------- | ------------------------- | ------------ | ----------------------------------- | ------- | -------------------------------------------------- |
| Exposure | Auto exposure             | `0x009a0901` | menu: 1 Manual, 3 Aperture priority | 3       | toggle "Auto" (3 ↔ 1)                              |
| Exposure | Exposure time             | `0x009a0902` | 3–2047 (1), units 100 µs            | 250     | slider, shown in ms; enabled only when manual      |
| Exposure | Gain                      | `0x00980913` | 0–255                               | 0       | slider                                             |
| Exposure | Dynamic framerate         | `0x009a0903` | bool                                | 0       | switch ("Allow lower fps in low light")            |
| Exposure | Backlight compensation    | `0x0098091c` | 0–1                                 | 1       | switch                                             |
| Exposure | Power line frequency      | `0x00980918` | menu: 0 Off, 1 50 Hz, 2 60 Hz       | 2       | segmented                                          |
| Focus    | Autofocus                 | `0x009a090c` | bool                                | 1       | switch                                             |
| Focus    | Focus                     | `0x009a090a` | 0–255 (5)                           | 0       | slider; enabled only when autofocus off            |
| Framing  | Zoom                      | `0x009a090d` | 100–500                             | 100     | slider, shown as 1.0×–5.0×                         |
| Framing  | Pan                       | `0x009a0908` | ±36000 (3600), arc-seconds          | 0       | slider, shown in degrees; **disabled at zoom 100** |
| Framing  | Tilt                      | `0x009a0909` | ±36000 (3600), arc-seconds          | 0       | as Pan                                             |
| Color    | Auto white balance        | `0x0098090c` | bool                                | 1       | switch                                             |
| Color    | White balance temperature | `0x0098091a` | 2000–7500 K (10)                    | 4000    | slider in K; enabled only when AWB off             |
| Color    | Brightness                | `0x00980900` | 0–255                               | 128     | slider                                             |
| Color    | Contrast                  | `0x00980901` | 0–255                               | 128     | slider                                             |
| Color    | Saturation                | `0x00980902` | 0–255                               | 128     | slider                                             |
| Color    | Sharpness                 | `0x0098091b` | 0–255                               | 128     | slider                                             |

Enabled/disabled state follows the V4L2 `INACTIVE` flag as reported by the driver (re-queried after every write), not app-side guesses. The Pan/Tilt rule above is an extra UI rule.

### 4.2 Extension-unit control (FOV only)

| Group   | Control       | XU GUID                                | Selector | Size   | Values                    | UI                        |
| ------- | ------------- | -------------------------------------- | -------- | ------ | ------------------------- | ------------------------- |
| Framing | Field of view | `49e40215-f434-47fe-b158-0e885023e51b` | 5        | 1 byte | 0 = 90°, 1 = 78°, 2 = 65° | segmented 90° / 78° / 65° |

- The unit ID is resolved **by GUID** at connect time by parsing `/sys/bus/usb/devices/<dev>/descriptors`
  (readable without root; on this BRIO it resolves to unit 10). It is never hardcoded.
- Shown only if the GUID is present and `GET_INFO` reports GET|SET.
- The XU write path in the backend accepts exactly one (GUID, selector) pair. There is no generic "raw XU" command.

### 4.3 Stream mode

- Enumerated at every connect from the device (`VIDIOC_ENUM_FMT` / `ENUM_FRAMESIZES` / `ENUM_FRAMEINTERVALS`),
  MJPEG only, never hardcoded. The list differs between USB 2 and USB 3 (FINDINGS §0).
- Shown as "4K · 30", "1080p · 60", etc. (4096×2160 shown as "4096 × 2160 · 30"). Default: 1920×1080 @ 30.
- If the USB link speed (`/sys/bus/usb/devices/<dev>/speed`) is 480: show a notice in the Format section:
  "Connected at USB 2. Use a USB 3 cable and port for 4K and 60 fps."
- Known quirk: 640×480 advertises 120 fps but delivers 60. The info overlay (§7.4) shows **measured** fps.

### 4.4 Persistence and restore

- The camera loses its settings on power loss. The app saves the user's control values (per camera serial) and
  re-applies them on every connect/replug, in dependency order (auto modes first, then manual values).
- Global "Reset all" and per-control reset restore the device default (`default_value`).

## 5. Capture

### 5.1 Stills

- In Photo mode, the shutter saves the **next frame's original MJPEG bytes**, unmodified (no decode/re-encode), at the current mode's resolution.
- Filename: `BRIO_YYYY-MM-DD_HH-MM-SS.jpg` (local time; append `_2`, `_3` on collision).
- Feedback: brief white flash over the viewport, thumbnail appears in the capture bar.

### 5.2 Video

- Container: **Matroska (.mkv)** for all qualities (stays readable if writing stops abruptly).
- Quality (setting; default **Original**):
  - **Original** (default): the camera's MJPEG frames passed through, no re-encode. Exact camera output. ~1 MB/frame at 4K
    (≈1.8 GB/min at 4K30).
  - **Compact**: AV1, `vaav1enc` (hardware) if present and it keeps the mode's size, else `svtav1enc`. Encoder
    is chosen at runtime by probing the GStreamer registry. If neither exists, Compact is hidden.
    Mesa's `vaav1enc` pads sizes it can't encode exactly without signalling a crop (1920×1080 → 1920×1082 with
    two black rows); it is only used for widths that are multiples of 64 and heights that are multiples of 16
    (4K, 1440p, 720p, 480p …). 1080p therefore uses `svtav1enc` (≈1.3 cores while recording).
- Audio toggle (mic icon), available in Video mode only:
  - Source is the **BRIO's own microphone**, never the system default (this machine has another USB mic). Match it by
    finding the ALSA card whose sysfs parent is the same USB device as the camera, then the PipeWire node with that
    `api.alsa.card`. Record via `pipewiresrc`, 48 kHz, encoded with Opus (128 kb/s).
  - If the BRIO mic isn't found, the toggle is disabled with a tooltip.
  - PipeWire must never fall back to another source (`node.dont-fallback`, `node.dont-reconnect`).
    If the mic fails to start, delivers nothing within 2 s, or errors mid-recording, the recording
    continues video-only and a toast says so.
- Mode picker and FOV (if FOV requires a stream restart, see §10) are locked while recording. Image controls stay live.
- Recording indicator: red dot + elapsed time in the status chip; shutter becomes a stop button.
- Filename as stills, `.mkv`.
- Failure handling: output folder not writable or disk full → recording stops cleanly, partial file kept, error toast. Free space is checked at start (refuse if < 1 GB) and every few seconds.

### 5.3 Output folders

Each media type goes to its own folder:

| Media | Default | Fallback |
|---|---|---|
| Photos | `xdg-user-dir PICTURES` (`dirs::picture_dir()`) | `~/Pictures` |
| Videos | `xdg-user-dir VIDEOS` (`dirs::video_dir()`) | `~/Videos` |

- Folders are created if missing. XDG lookup handles localized folder names.
- Each folder is changed independently in Settings → Output via the native folder picker (`tauri-plugin-dialog`),
  with a per-folder reset to the XDG default.
- Thumbnail click opens the file with the default app; "Show in folder" via `tauri-plugin-opener`.

### 5.4 App preferences

- Stored in `$XDG_CONFIG_HOME/camera-settings/config.toml`: photo and video folders, mode, video quality, audio on/off, last Photo/Video mode, saved control values keyed by camera serial.

## 6. Connection model and hot-unplug (the GNOME Camera problem)

### 6.1 Device discovery

- Find the camera by VID:PID `046d:085e` via sysfs, never by `/dev/videoN` (the node numbers change between replugs:
  observed `video0` → `video1`). The main capture node is the one with `index` = 0 on that USB device. The IR node (index 2) is ignored in v1.
- Hotplug via a `udev` monitor (subsystems `video4linux` and `usb`) on a dedicated thread.

### 6.2 States

```
NoCamera ──add──▶ Connecting ──first frame──▶ Streaming ⇄ Recording
   ▲                  │  │                       │
   │              EBUSY  error                   │ remove / error / watchdog
   │                  ▼  ▼                       ▼
   └───────────── Busy   Error ◀──────────── (teardown) ──▶ NoCamera
```

Plus window visibility (§6.5):

```
Streaming ──window minimized/hidden──▶ Paused (device released) ──window restored──▶ Connecting
```

- **NoCamera**: no BRIO present, or it was removed.
- **Paused**: window minimized/hidden while not recording. The pipeline is torn down and the device fd closed.
  Hotplug events are still processed, so an unplug while Paused moves to NoCamera.
- **Busy**: the device exists but streaming fails with `EBUSY` (another app holds it). Retry every 2 s.
- **Error**: anything else; show message, retry with backoff (1, 2, 5, 10 s).

### 6.3 Rules

1. **The UI thread does no device I/O.** All ioctls, pipeline state changes and file I/O happen on backend worker threads. Tauri commands return immediately or are `async` with a timeout (1 s for controls).
2. **Teardown never blocks anything visible.** Pipeline shutdown (`set_state(Null)`) runs on a dedicated thread with a 3 s timeout. If it doesn't finish, the pipeline is abandoned (leaked) and a fresh one is built on reconnect.
3. **Watchdog**: if no frame arrives for 2 s while Streaming/Recording, treat the camera as gone (covers cases where neither an error nor a udev event arrives).
4. **Recording on unplug**: the recording branch is sent EOS so Matroska is finalized. The file is kept and a toast says "Recording saved (camera disconnected)".
5. **Control writes on a vanished device** fail fast (`ENODEV`) and are dropped silently; the UI is already showing the disconnected state.
6. **Replug**: a udev add (or periodic rescan) triggers Connecting, restores mode and saved controls (§4.4), and resumes preview within 3 s of the device enumerating.
7. The camera-off screen (§7.3) is rendered entirely by the frontend, so it keeps working even if the backend is stuck.

### 6.4 Acceptance tests (all must pass)

| #   | Scenario                              | Expected                                                                             |
| --- | ------------------------------------- | ------------------------------------------------------------------------------------ |
| H1  | Launch with no camera                 | Camera-off screen within 1 s; UI responsive                                          |
| H2  | Unplug during preview                 | Camera-off screen within 2 s; window stays responsive (drag, resize, open settings)  |
| H3  | Unplug while dragging a slider        | No freeze; slider stops affecting anything; camera-off screen                        |
| H4  | Unplug during still capture           | No file or a valid JPEG; never a zero-byte/corrupt file left behind                  |
| H5  | Unplug during 4K recording with audio | Playable `.mkv` up to the unplug point; interrupted toast                            |
| H6  | Replug after any of the above         | Preview resumes with previous mode and settings, no user action                      |
| H7  | Camera held by another app            | Busy screen; resumes automatically when released                                     |
| H8  | Suspend/resume with camera attached   | Preview resumes (or clean reconnect) after resume                                    |
| H9  | Unplug/replug 20× in a row            | No crash, no leaked threads growing unbounded, no fd leak beyond abandoned pipelines |
| H10 | Minimize while previewing             | Within 2 s no process holds the device (`fuser /dev/videoN` empty) and the camera LED is off |
| H11 | Restore after H10                     | Preview resumes within 1.5 s with the same mode and settings                          |
| H12 | Minimize while recording              | Recording and audio continue uninterrupted; stream stops only after recording stops   |

Test aid: simulate unplug without touching the cable with
`echo 0 | sudo tee /sys/bus/usb/devices/<dev>/authorized` (and `1` to replug).

### 6.5 Window visibility

- When the window is minimized or hidden and **not recording**, stop streaming and release the device completely, so
  the camera's LED turns off and other apps can use the camera.
- On restore, reconnect (mode and controls re-applied) and show black, not static, until the first frame arrives.
  Static means "no camera".
- While recording, minimizing changes nothing. If the window is still minimized when recording stops, enter Paused.
- Detection: Tauri window events plus the webview's `visibilitychange`. GNOME on Wayland may not report minimize
  reliably (M0 checks this). If it doesn't, the fallback is a manual pause/resume button in the status chip
  (Lucide `pause` / `play`).

## 7. User interface

### 7.1 Principles

Dark only (camera apps compete with the image). Full-bleed black viewport, video letterboxed (`object-fit: contain`). Controls float over the video on translucent, blurred surfaces and fade out after 3 s of no mouse movement (not while recording, not while a panel is open). One accent color for recording (red). System UI font. Default window 1280×800, minimum 640×420.

### 7.2 Layout

```
┌──────────────────────────────────────────────────────────────┐
│ [● 00:12  4K · 30]                                [⚙]          │  status chip (left), settings button (right)
│                                                                │
│                         live video                             │
│                                                                │
│               [▣ thumb]   [ Photo | Video ]   ( ◉ )   [🎤]      │  capture bar
└──────────────────────────────────────────────────────────────┘
```

- **Capture bar** (bottom center): last-capture thumbnail · Photo/Video segmented toggle · shutter · mic toggle (Video mode only).
- **Settings drawer** (right, 340 px, overlays the video): sections **Format** (mode list, USB 2 notice),
  **Framing** (FOV, zoom, pan, tilt), **Exposure**, **Focus**, **Color**, **Output** (photo folder, video folder, video quality),
  **Camera info** (model, serial, firmware = `bcdDevice`, USB link speed; all from sysfs). "Reset all" at the bottom. Each slider shows its value with units and has a reset icon when off-default; double-click also resets.
- **Toasts** (bottom, above capture bar): saved file (with "Show in folder"), errors.

### 7.3 Camera-off / busy screen

- Full viewport: animated **video static** (TV-noise) with a large centered Lucide `video-off` icon (96 px, white, soft shadow) and one line of text below: "Camera disconnected" / "Camera in use by another app" / error text.
- Static: canvas at ~320×180, random grayscale per pixel, scaled up with slight blur, faint horizontal scanlines and a slow rolling band; ~24 fps. Kept dim (~35% luminance) so the icon reads clearly.
- `prefers-reduced-motion` → plain black background, no animation.
- Capture bar and mode/control widgets are disabled (controls keep showing last values).

### 7.4 Info overlay (toggle, `I` key)

Small monospace panel: resolution, format, **measured** fps, dropped frames, USB speed, recording bitrate.

### 7.5 Icons (Lucide)

| Use                            | Icon                             |
| ------------------------------ | -------------------------------- |
| Camera off / busy              | `video-off`                      |
| Photo mode / shutter (photo)   | `camera`                         |
| Video mode                     | `video`                          |
| Record / stop                  | `circle` (filled red) / `square` |
| Mic on / off                   | `mic` / `mic-off`                |
| Settings                       | `sliders-horizontal`             |
| Close drawer                   | `x`                              |
| Reset                          | `rotate-ccw`                     |
| Output folder / show in folder | `folder-open`                    |
| Info overlay                   | `info`                           |
| Pause / resume (fallback §6.5) | `pause` / `play`                 |
| Quit                           | `power`                          |
| USB notice                     | `usb`                            |
| FOV                            | `scan`                           |
| Zoom                           | `zoom-in`                        |
| Exposure / brightness          | `sun`                            |
| Focus                          | `focus`                          |
| White balance                  | `thermometer`                    |
| Error                          | `triangle-alert`                 |

### 7.6 Keyboard

`Space` shutter / start-stop recording · `P` / `V` switch Photo/Video · `M` mic toggle · `S` or `Ctrl+,` settings ·
`Esc` close drawer · `I` info overlay · `F` / `F11` fullscreen · `Ctrl+Q` / `Ctrl+W` quit.

A Quit button (top right, next to Settings) is always shown: some compositors (e.g. niri) draw no
window decorations. Quitting while recording stops and finalizes the recording first.
The webview's context menu and web inspector are disabled unless the app is started with `--debug`.

## 8. Architecture

### 8.1 Overview

```
 ┌──────────── Rust backend (src-tauri) ───────────────────────────────┐
 │ device::discovery   sysfs scan, serial, link speed, XU GUID→unit map │
 │ device::hotplug     udev monitor thread → events                     │
 │ controls::v4l2      QUERY_EXT_CTRL / G/S_EXT_CTRLS via nix ioctls    │
 │ controls::xu        UVCIOC_CTRL_QUERY, FOV only                      │
 │ controls::allowlist the table in §4                                  │
 │ pipeline            GStreamer: v4l2src(MJPEG) → tee ─┬→ preview      │
 │                                                      ├→ still tap    │
 │                                                      └→ record bin   │
 │                     pipewiresrc(BRIO mic) → opusenc ─→ record bin    │
 │ session             state machine §6, watchdog, teardown worker      │
 │ config              TOML prefs + saved controls                      │
 └─────────────── commands / events / frame channel ───────────────────┘
 ┌──────────── Frontend (Svelte 5 + TS, @lucide/svelte) ───────────────┐
 │ Viewport (canvas) · StaticNoise · CaptureBar · SettingsDrawer · Toast│
 └──────────────────────────────────────────────────────────────────────┘
```

Key crates: `tauri` 2, `gstreamer`/`gstreamer-app`, `udev`, `nix` (ioctls), `dirs`, `serde`/`toml`, `thiserror`,
`tracing`, `tauri-plugin-dialog`, `tauri-plugin-opener`.

Code layout (Cargo workspace, all crates `publish = false`):

```
camera-settings/
├─ Cargo.toml                 workspace
├─ crates/camera-control/     library: discovery, hotplug, allowlisted V4L2 + UVC XU controls (no Tauri, no GStreamer)
├─ src-tauri/                 app crate `camera-settings`: session, pipeline, config, Tauri commands
└─ src/                       Svelte frontend
```

### 8.2 Preview transport

- Preview branch: `queue leaky=downstream max-size-buffers=1` → (if mode > 1080p: `jpegdec` (not `vajpegdec`, see §13) →
  `videoscale` to ≤ 1920 wide → `jpegenc quality=85`; else pass through the camera JPEG) → `appsink drop=true max-buffers=1`.
- Frames go to the frontend over a Tauri `ipc::Channel` as raw bytes. The frontend decodes with `createImageBitmap`
  and draws to a canvas. **Latest frame only**: if a frame arrives while one is decoding, the older one is dropped.
- Fallback if M0 shows the channel can't keep up: a loopback-only HTTP server on a random port serving
  `multipart/x-mixed-replace` MJPEG with a per-session token, shown via `<img>`.

### 8.3 Backend ↔ frontend API

Commands:
`get_snapshot()` (state, device info, modes, controls, prefs) · `set_mode(mode)` · `set_control(id, value)` ·
`reset_control(id)` · `reset_all()` · `capture_still()` · `start_recording({ audio, quality })` ·
`stop_recording()` · `set_prefs(partial)` · `reveal(path)` · `open_preview(channel)`.

Events:
`camera-state` {state, message?} · `controls-changed` {controls[]} · `modes-changed` {modes[], usbSpeed} ·
`recording` {elapsedMs, bytes} · `capture-saved` {path, kind, interrupted} · `stats` {fps, dropped, resolution} ·
`error` {message}.

Control model sent to the UI: `{ id, label, group, kind: "int"|"bool"|"menu", min, max, step, default, value,
options?, unit?, inactive, readOnly }`. FOV appears as `{ id: "fov", kind: "menu", options: [90, 78, 65] }`.

## 9. Quality targets

- Preview latency (glass-to-glass) ≤ 150 ms at 1080p30; preview never stutters the UI.
- CPU (whole app) ≤ 25% of one core at 1080p30 preview; ≤ 100% of one core at 4K30 preview + Original recording.
- A/V sync offset in recordings ≤ 1 frame (33 ms at 30 fps); check with a clap test.
- Cold start to first frame ≤ 1.5 s.

## 10. Milestones

**M0: spike (answer the unknowns before building UI)**

- Tauri 2 skeleton + GStreamer preview over `ipc::Channel` on WebKitGTK at 1080p60 and 4K30: measure latency,
  CPU, dropped frames vs. §9. Decide on the channel or the HTTP fallback.
- Does setting FOV take effect while streaming, or does it need a stream restart? (Decides the lock rule in §5.2.)
- Does the BRIO keep control values across a power cycle? (Expected: no → §4.4 restore.)
- Unplug behaviour of `v4l2src`: which bus message/error arrives, does `set_state(Null)` block? (Validates §6.3.)
- Can minimize/hide be detected reliably on GNOME Wayland via Tauri window events or `visibilitychange`? (Decides
  whether the §6.5 pause button is needed.)

**M1**: discovery, udev hotplug, state machine, camera-off screen with static, H1/H2/H6/H7 passing.
**M2**: controls (allowlist, FOV via GUID lookup), settings drawer, persistence + restore, H3.
**M3**: stills, photo/video output folders, thumbnails/toasts, H4.
**M4**: video recording, Original + Compact, BRIO mic, A/V sync, H5.
**M5**: polish: auto-hide controls, keyboard shortcuts, info overlay, minimize-to-pause, H8–H12, `.rpm` + `.deb` via
Tauri bundler.

## 11. Future (not v1)

- IR camera view (the 340×340 greyscale stream; IR LEDs strobe on alternate frames).
- More Logitech controls (HDR, RightLight…) once identified via a USB capture of Logi Tune.
- Raw YUYV/NV12 modes; photo at maximum resolution independent of preview mode.

## 12. Open questions

1. Final product name. Placeholder: **Camera Settings** / `camera-settings`. The Tauri bundle `identifier`
   (reverse-DNS, used for the desktop entry and packages) should be chosen together with the final name.
   Until then use `io.github.camera-settings.CameraSettings`.

## 13. Decisions

| Date | Decision |
|---|---|
| 2026-10-21 | Default video quality: **Original** (MJPEG passthrough, `.mkv`). Compact (AV1) is opt-in. |
| 2026-10-21 | Photos → XDG Pictures, videos → XDG Videos; each folder configurable (§5.3). |
| 2026-10-21 | Stop streaming and release the camera when the window is minimized/hidden, except while recording (§6.5). |
| 2026-10-21 | No Flatpak/Snap/AppImage and no crates.io publishing for v1; `.rpm`/`.deb` + source builds (§3.1). |
| 2026-10-21 | The privacy LED is never touched by the app (§2). |
| 2026-10-21 | M0: preview stays on the `ipc::Channel` (raw bytes, frontend acks each frame, at most 2 in flight). 1080p30/60 and 4K30 deliver every frame with none skipped; no HTTP fallback needed. |
| 2026-10-21 | M0: **never use `vajpegdec`.** On the Radeon 890M it hangs the GPU's JPEG ring ("context is lost … guilty of a hard recovery") on the BRIO's 4:2:2 MJPEG. Preview transcode and Compact recording decode with `jpegdec`. `vaav1enc` (NV12 input) is fine. |
| 2026-10-21 | M0: FOV takes effect live while streaming (no restart, fps unchanged), so FOV stays unlocked while recording. |
| 2026-10-21 | M0: on unplug `v4l2src` posts `Could not read from resource` within ms of the udev event, and `set_state(Null)` returns promptly. The recording is finalized by pushing EOS into the record branch (534 s 4K file finalized 25 ms after unplug). |
| 2026-10-21 | M0 measured CPU (% of one core, app + all WebKit processes): 1080p30 preview ≈ 33% (**misses** the 25% target; WebKit's JPEG decode is ~25% by itself); 4K30 preview ≈ 97%; 4K30 preview + Original recording ≈ 112% (**misses** the 100% target). An HTTP/`<img>` fallback would not help (same decode). Follow-up: size the preview to the window. |
| 2026-10-21 | A/V sync (clap test, Compact 1080p): audio trails video by ≈55 ms in the file (target ≤ 33 ms). Not noticeable in Video Player or Cine; deferred. Likely cause: PipeWire buffer timestamps vs. UVC capture timestamps. |
