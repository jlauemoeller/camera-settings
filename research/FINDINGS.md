# Logitech BRIO on Linux — what the device exposes

Device: `046d:085e` "Logitech BRIO", firmware bcdDevice 0.17, UVC 1.00, kernel 7.2.11 (Fedora 44). All data below comes from `lsusb -v` and `probe.py` (GET-only ioctls; nothing was written to the camera).

## 0. Connection speed decides which modes exist

The BRIO only advertises its high modes when enumerated at SuperSpeed. The device-node numbers (`/dev/videoN`) also shift between replugs, so check `/sys/class/video4linux/*/name` and `index` (index 0 = main camera, 2 = IR).

|                           | USB 2 (480 Mbps, non-SS cable) | USB 3 (5 Gbps, original blue-tongue cable)                                |
| ------------------------- | ------------------------------ | ------------------------------------------------------------------------- |
| MJPEG max                 | 1920×1080 @ 30                 | 4096×2160 / 3840×2160 / 2560×1440 @ 30, 1080p @ 60, 720p @ 90, 480p @ 120 |
| YUYV (uncompressed) 1080p | 5 fps                          | 30 fps                                                                    |

All 4K/1440p modes are **MJPEG only**. Apps that prefer raw YUYV (e.g. GNOME Camera/Snapshot via PipeWire, which picked YUY2 1920×1080 @ 30) will not use them unless they request MJPEG.

### Measured at USB 3 (camera timestamps, 8 s per mode, auto exposure)

| Mode (MJPEG)         | Advertised    | Delivered                                                                                    |
| -------------------- | ------------- | -------------------------------------------------------------------------------------------- |
| 4096×2160, 3840×2160 | 30            | 30.0                                                                                         |
| 1920×1080            | 60            | 60.0                                                                                         |
| 1280×720             | 90            | 90.0                                                                                         |
| 640×480              | 120 (also 90) | **60.0**, even with dynamic framerate off and a 5 ms manual exposure (likely a firmware cap) |

4K is genuine extra detail, not upscaling (see `captures/compare_4k_vs_1080p.png`).

**IR camera** (`gray` 340×340 @ 30): it streams without any extension-unit setup. The IR emitter strobes on **alternate frames** (mean luma e.g. 12 / 93 / 12 / 93 …), giving lit + ambient pairs like Windows Hello uses. Clothing and skin look very different
in IR than in RGB.

## 1. USB interfaces

| If  | Class           | Linux node                               | What it is                                                                                                                |
| --- | --------------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| 0   | Video Control   | (control path for all nodes)             | Units/terminals, incl. 8 extension units                                                                                  |
| 1   | Video Streaming | `/dev/video0` (+ `/dev/video1` metadata) | Main RGB camera: YUYV + MJPEG, up to 1920×1080 at USB 2                                                                   |
| 2   | Video Streaming | `/dev/video2` (+ `/dev/video3` metadata) | **IR camera** (Windows Hello): frame-based, format GUID `00000032-…` = D3DFMT_L8 (8-bit grey), 340×340 / 440×440 @ 30 fps |
| 3/4 | Audio           | ALSA card                                | Microphone: only Mute + Volume feature controls (no hidden audio extensions)                                              |

## 2. Standard UVC controls (already accessible via V4L2)

The kernel maps every control the Camera Terminal and Processing Unit advertise. None of the standard set is missing.

Brightness, Contrast, Saturation, Sharpness, Gain (0–255) · White Balance auto / temperature 2000–7500 K · Backlight compensation · Power line frequency (off/50/60 Hz) · Auto exposure (manual / aperture priority) · Exposure time 3–2047 (100 µs units) · Dynamic framerate · Autofocus / absolute focus 0–255 · Digital Zoom 100–500 · Digital Pan/Tilt ±36000 step 3600 (only meaningful when zoomed).

## 3. Extension units (vendor controls, need raw `UVCIOC_CTRL_QUERY`)

Confidence: **confirmed** = matches Linux kernel source or `cameractrls`; **inferred** = guessed from data; **unknown**.

### Unit 10 `{49e40215-f434-47fe-b158-0e885023e51b}`: Logitech BRIO video controls

- **sel 5: Field of view** (confirmed, cameractrls `LOGITECH_BRIO_FOV_SEL`). 1 byte: `0`=90°, `1`=78°, `2`=65°. Current: 90°.
- sel 2, 4, 12, 14: 1-byte booleans (unknown; candidates are HDR, RightLight, etc. Needs capture, see §5).
- sel 9 (2 B, max `01 ff`), sel 11 (2 B, 1–20), sel 8 (6 B, max `0x2ee0,0x2ee0,2,255`): unknown, settable.
- sel 6 (fixed `01 00`), sel 10 (RO `01 00`), sel 15 (RO `0x0777, 0x0378` = 1911, 888): unknown.

### Unit 11 `{ffe52d21-8030-4e2c-82d9-f587d00540bd}`: Logitech Peripheral

- **sel 9: LED** (confirmed, kernel `UVC_GUID_LOGITECH_PERIPHERAL`, cameractrls). 5 bytes; byte 1 = mode (0 off, 1 on, 2 blink, 3 auto), byte 3 = blink frequency (0.05 Hz units).
  **Blocked by the kernel** (`EACCES`, "Privacy related controls can only be accessed if module parameter allow_privacy_override is true"). To allow it:
  `echo 1 | sudo tee /sys/module/uvcvideo/parameters/allow_privacy_override` (runtime), or persist with `options uvcvideo allow_privacy_override=1` in `/etc/modprobe.d/uvcvideo.conf`.
- sel 16, 17, 18: 1-byte booleans (unknown; sel 16 currently 1).
- sel 15: 16-byte fixed blob (unknown).
- Pan/tilt-relative selectors (1, 2) that the kernel has built-in mappings for are *not* advertised by this camera.

### Unit 12 `{0f3f95dc-2632-4c4e-92c9-a04782f43bc8}`: Microsoft MSXU 1.5 (confirmed, kernel `UVC_GUID_MSXU_1_5`)

- sel 6: `MSXU_CONTROL_FACE_AUTHENTICATION`, which switches the IR face-auth mode (Windows Hello).
- sel 9: `MSXU_CONTROL_METADATA`, RO.

### Unit 8 `{69678ee4-410f-40db-a850-7420d7d8240e}`: device info (inferred), all read-only

- sel 6 = `6d 04 5e 08 17 00` → VID 046d, PID 085e, bcdDevice 0017.
- sel 10 = `0x1070, 0x0c30` → 4208×3120 (looks like the 13 MP sensor's native resolution).
- sel 1 = `00 02 44 00` (probably the firmware version), sel 2/9 = `0xf21c`, others unknown.

### Unit 21 `{5a6d654c-7e35-4d4e-810d-069d15e0f79b}`: 1 RO selector

- sel 1 = `0x0200, 0x0500, 0x02d0`, i.e. contains 1280×720 (inferred: some stream/sensor-mode report).

### ⚠️ Units 6, 9, 14: likely firmware/debug channels. Do not write to these.

- **Unit 6** `{23e49ed0-1178-4f31-ae52-d2fb8a8d3b48}`: many SET-only 8/16/32-byte buffers plus GET-only reply
  buffers. This looks like a command/response mailbox, possibly firmware update. sel 10 reads ASCII `"PYTHON_V2B"`.
- **Unit 9** `{1f5d4ca9-de11-4487-840d-50933c8ec8d1}`: 18 selectors with wide-open 0x00–0xFF ranges and
  address/value-shaped fields. This looks like register peek/poke (test/debug).
- **Unit 14** `{2c49d16a-32b8-4485-3ea8-643a152362f2}`: sel 1/2 return `ENOENT`, and sel 3–6 return values that change on every read and
  fall outside their own min/max. Unknown purpose.

## 4. What this means for an app

Easy now: a GUI over the standard V4L2 controls, plus **FOV** (unit 10 sel 5), which works today without root. LED control works after enabling `allow_privacy_override`. Optional: an IR preview from `/dev/video2`.

Linux tools that already do some of this: `v4l2-ctl` / `qv4l2` (`sudo dnf install v4l-utils`), `guvcview`, and `cameractrls` (github.com/soyersoyer/cameractrls), which already knows BRIO FOV + Logitech LED.

## 5. Identifying the unknown controls (HDR, RightLight, …)

Do not guess by writing values. Instead, capture what Logitech's own software sends: on Windows, use USBPcap + Wireshark (or on macOS, a USB capture) while toggling each setting in Logi Tune / G HUB. Each change shows up as a `SET_CUR` class request to `wIndex = (unit << 8) | interface 0` with `wValue = selector << 8`, which tells you the exact unit/selector/payload. Then add that entry to the app.

## Reproducing

```
python3 research/probe.py /dev/video1          # all controls + all XUs (main node = the one whose sysfs index is 0)
python3 research/probe.py /dev/video1 10 11    # just specific units
```
