//! An open control handle on the camera's main node: allowlisted controls and stream modes.
//!
//! The handle never starts streaming, so it can be held next to a GStreamer pipeline. It must be
//! dropped (closing the fd) whenever the app releases the camera.

use std::fs::OpenOptions;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use nix::errno::Errno;
use serde::Serialize;

use crate::allowlist::{self, FOV, Group, V4l2Def};
use crate::discovery::CameraInfo;
use crate::sys::{self, cstr, zeroed};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("control `{0}` is not allowed")]
    NotAllowed(String),
    #[error("control `{0}` is not available on this camera")]
    Unavailable(String),
    #[error("value {value} out of range for `{id}`")]
    OutOfRange { id: String, value: i64 },
    #[error("camera disconnected")]
    Gone,
    #[error("camera busy")]
    Busy,
    #[error("{op} failed: {errno}")]
    Ioctl { op: &'static str, errno: Errno },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    fn ioctl(op: &'static str, errno: Errno) -> Self {
        match errno {
            Errno::ENODEV | Errno::ENXIO | Errno::ESHUTDOWN | Errno::EPIPE => Error::Gone,
            Errno::EBUSY => Error::Busy,
            _ => Error::Ioctl { op, errno },
        }
    }

    pub fn is_gone(&self) -> bool {
        match self {
            Error::Gone => true,
            Error::Io(e) => matches!(e.raw_os_error(), Some(libc::ENODEV | libc::ENOENT | libc::ENXIO)),
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Int,
    Bool,
    Menu,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MenuOption {
    pub value: i64,
    pub label: String,
}

/// The control model sent to the UI (SPEC §8.3).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Control {
    pub id: &'static str,
    pub label: &'static str,
    pub group: Group,
    pub kind: Kind,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub default: i64,
    pub value: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<MenuOption>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<&'static str>,
    pub inactive: bool,
    pub read_only: bool,
}

impl Control {
    fn accepts(&self, value: i64) -> bool {
        match &self.options {
            Some(opts) => opts.iter().any(|o| o.value == value),
            None => (self.min..=self.max).contains(&value),
        }
    }
}

/// An MJPEG stream mode. Frame rate is `fps_n / fps_d` (the inverse of the V4L2 interval).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub fps_n: u32,
    pub fps_d: u32,
}

impl Mode {
    pub const DEFAULT: Mode = Mode {
        width: 1920,
        height: 1080,
        fps_n: 30,
        fps_d: 1,
    };

    pub fn fps(&self) -> f64 {
        f64::from(self.fps_n) / f64::from(self.fps_d.max(1))
    }
}

pub struct Device {
    fd: OwnedFd,
    fov_unit: Option<u8>,
}

impl Device {
    /// Open the main node read/write, non-blocking. Does not start streaming.
    pub fn open(info: &CameraInfo) -> Result<Self> {
        let mut dev = Self::open_path(&info.node)?;
        dev.fov_unit = info.unit_for_guid(FOV.guid);
        Ok(dev)
    }

    pub fn open_path(node: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(node)?;
        let dev = Device {
            fd: file.into(),
            fov_unit: None,
        };
        let mut cap: sys::v4l2_capability = zeroed();
        // SAFETY: valid fd and a correctly sized struct.
        unsafe { sys::vidioc_querycap(dev.raw(), &mut cap) }.map_err(|e| Error::ioctl("QUERYCAP", e))?;
        Ok(dev)
    }

    fn raw(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    /// Every allowlisted control the device exposes, with current values and flags.
    pub fn controls(&self) -> Result<Vec<Control>> {
        let mut out = Vec::with_capacity(allowlist::V4L2_CONTROLS.len() + 1);
        if let Some(fov) = self.query_fov()? {
            out.push(fov);
        }
        for def in allowlist::V4L2_CONTROLS {
            if let Some(c) = self.query_v4l2(def)? {
                out.push(c);
            }
        }
        Ok(out)
    }

    pub fn control(&self, id: &str) -> Result<Control> {
        if id == FOV.id {
            return self.query_fov()?.ok_or_else(|| Error::Unavailable(id.into()));
        }
        let def = allowlist::v4l2_by_id(id).ok_or_else(|| Error::NotAllowed(id.into()))?;
        self.query_v4l2(def)?.ok_or_else(|| Error::Unavailable(id.into()))
    }

    /// Write one allowlisted control. The value is validated against the device-reported range.
    pub fn set(&self, id: &str, value: i64) -> Result<()> {
        let ctrl = self.control(id)?;
        if ctrl.read_only {
            return Err(Error::NotAllowed(id.into()));
        }
        if !ctrl.accepts(value) {
            return Err(Error::OutOfRange { id: id.into(), value });
        }
        if id == FOV.id {
            return self.set_fov(value as u8);
        }
        let def = allowlist::v4l2_by_id(id).expect("checked by control()");
        self.write_v4l2(def.cid, value)
    }

    /// The stream's current frame size. Read-only, so it also works (and reports the other app's
    /// format) while another process owns the stream.
    pub fn current_size(&self) -> Result<(u32, u32)> {
        let mut f: sys::v4l2_format = zeroed();
        f.type_ = sys::V4L2_BUF_TYPE_VIDEO_CAPTURE;
        // SAFETY: valid fd and a correctly sized struct.
        unsafe { sys::vidioc_g_fmt(self.raw(), &mut f) }.map_err(|e| Error::ioctl("G_FMT", e))?;
        Ok((f.width, f.height))
    }

    /// Is another process streaming from (or has it claimed) this camera? Asks for zero buffers:
    /// the driver refuses with `EBUSY` if another file handle owns the stream, and otherwise
    /// allocates nothing and leaves no claim behind. Never disturbs the other stream.
    pub fn stream_in_use(&self) -> Result<bool> {
        let mut rb: sys::v4l2_requestbuffers = zeroed();
        rb.type_ = sys::V4L2_BUF_TYPE_VIDEO_CAPTURE;
        rb.memory = sys::V4L2_MEMORY_MMAP;
        // SAFETY: valid fd and a correctly sized struct.
        match unsafe { sys::vidioc_reqbufs(self.raw(), &mut rb) } {
            Ok(_) => Ok(false),
            Err(Errno::EBUSY) => Ok(true),
            Err(e) => Err(Error::ioctl("REQBUFS", e)),
        }
    }

    // --- V4L2 ---------------------------------------------------------------------------

    fn query_v4l2(&self, def: &V4l2Def) -> Result<Option<Control>> {
        let mut q: sys::v4l2_query_ext_ctrl = zeroed();
        q.id = def.cid;
        // SAFETY: valid fd and a correctly sized struct.
        match unsafe { sys::vidioc_query_ext_ctrl(self.raw(), &mut q) } {
            Ok(_) => {}
            Err(Errno::EINVAL) => return Ok(None),
            Err(e) => return Err(Error::ioctl("QUERY_EXT_CTRL", e)),
        }
        if q.flags & sys::V4L2_CTRL_FLAG_DISABLED != 0 {
            return Ok(None);
        }
        let kind = match q.type_ {
            sys::V4L2_CTRL_TYPE_INTEGER => Kind::Int,
            sys::V4L2_CTRL_TYPE_BOOLEAN => Kind::Bool,
            sys::V4L2_CTRL_TYPE_MENU => Kind::Menu,
            _ => return Ok(None),
        };
        let value = if q.flags & sys::V4L2_CTRL_FLAG_WRITE_ONLY != 0 {
            q.default_value
        } else {
            self.read_v4l2(def.cid)?
        };
        let options = (kind == Kind::Menu).then(|| self.menu_options(def.cid, q.minimum, q.maximum));
        Ok(Some(Control {
            id: def.id,
            label: def.label,
            group: def.group,
            kind,
            min: q.minimum,
            max: q.maximum,
            step: q.step.max(1) as i64,
            default: q.default_value,
            value,
            options,
            unit: def.unit,
            inactive: q.flags & sys::V4L2_CTRL_FLAG_INACTIVE != 0,
            read_only: q.flags & sys::V4L2_CTRL_FLAG_READ_ONLY != 0,
        }))
    }

    fn menu_options(&self, cid: u32, min: i64, max: i64) -> Vec<MenuOption> {
        (min.max(0)..=max.min(64))
            .filter_map(|i| {
                let mut m: sys::v4l2_querymenu = zeroed();
                m.id = cid;
                m.index = i as u32;
                // SAFETY: valid fd and a correctly sized struct.
                unsafe { sys::vidioc_querymenu(self.raw(), &mut m) }.ok()?;
                let name = m.name;
                Some(MenuOption {
                    value: i,
                    label: cstr(&name),
                })
            })
            .collect()
    }

    fn ext_ctrls(&self, cid: u32, value: i64, write: bool) -> Result<i64> {
        let mut ctrl = sys::v4l2_ext_control {
            id: cid,
            size: 0,
            reserved2: 0,
            value64: value,
        };
        let mut ctrls = sys::v4l2_ext_controls {
            which: sys::V4L2_CTRL_WHICH_CUR_VAL,
            count: 1,
            error_idx: 0,
            request_fd: 0,
            reserved: 0,
            controls: &mut ctrl,
        };
        // SAFETY: valid fd; `ctrls.controls` points at one live control for the call's duration.
        let res = unsafe {
            if write {
                sys::vidioc_s_ext_ctrls(self.raw(), &mut ctrls)
            } else {
                sys::vidioc_g_ext_ctrls(self.raw(), &mut ctrls)
            }
        };
        res.map_err(|e| Error::ioctl(if write { "S_EXT_CTRLS" } else { "G_EXT_CTRLS" }, e))?;
        let v = ctrl.value64;
        // 32-bit controls only fill the low half of the union.
        Ok(i64::from(v as i32))
    }

    fn read_v4l2(&self, cid: u32) -> Result<i64> {
        self.ext_ctrls(cid, 0, false)
    }

    fn write_v4l2(&self, cid: u32, value: i64) -> Result<()> {
        self.ext_ctrls(cid, value, true).map(drop)
    }

    // --- Extension unit (FOV only) --------------------------------------------------------

    fn xu_query(&self, unit: u8, query: u8, data: &mut [u8]) -> Result<()> {
        let mut q = sys::uvc_xu_control_query {
            unit,
            selector: FOV.selector,
            query,
            size: data.len() as u16,
            data: data.as_mut_ptr(),
        };
        // SAFETY: valid fd; `q.data` points at `q.size` writable bytes for the call's duration.
        unsafe { sys::uvcioc_ctrl_query(self.raw(), &mut q) }.map_err(|e| Error::ioctl("UVCIOC_CTRL_QUERY", e))?;
        Ok(())
    }

    fn query_fov(&self) -> Result<Option<Control>> {
        let Some(unit) = self.fov_unit else { return Ok(None) };
        let mut info = [0u8; 1];
        match self.xu_query(unit, sys::UVC_GET_INFO, &mut info) {
            Ok(()) => {}
            Err(e @ Error::Gone) => return Err(e),
            Err(_) => return Ok(None),
        }
        if info[0] & (sys::UVC_INFO_GET | sys::UVC_INFO_SET) != sys::UVC_INFO_GET | sys::UVC_INFO_SET {
            return Ok(None);
        }
        let mut len = [0u8; 2];
        self.xu_query(unit, sys::UVC_GET_LEN, &mut len)?;
        if u16::from_le_bytes(len) != FOV.size {
            return Ok(None);
        }
        let mut cur = [0u8; 1];
        self.xu_query(unit, sys::UVC_GET_CUR, &mut cur)?;
        let mut def = [0u8; 1];
        let default = match self.xu_query(unit, sys::UVC_GET_DEF, &mut def) {
            Ok(()) => i64::from(def[0]),
            Err(_) => FOV.options[0].0,
        };
        Ok(Some(Control {
            id: FOV.id,
            label: FOV.label,
            group: FOV.group,
            kind: Kind::Menu,
            min: FOV.options.iter().map(|o| o.0).min().unwrap_or(0),
            max: FOV.options.iter().map(|o| o.0).max().unwrap_or(0),
            step: 1,
            default,
            value: i64::from(cur[0]),
            options: Some(
                FOV.options
                    .iter()
                    .map(|&(value, l)| MenuOption { value, label: l.into() })
                    .collect(),
            ),
            unit: None,
            inactive: false,
            read_only: false,
        }))
    }

    fn set_fov(&self, value: u8) -> Result<()> {
        let unit = self.fov_unit.ok_or_else(|| Error::Unavailable(FOV.id.into()))?;
        let mut data = [value];
        self.xu_query(unit, sys::UVC_SET_CUR, &mut data)
    }

    // --- Stream modes ---------------------------------------------------------------------

    /// All discrete MJPEG modes, largest resolution first, then highest frame rate.
    pub fn modes(&self) -> Result<Vec<Mode>> {
        let mut out = Vec::new();
        if !self.has_mjpeg()? {
            return Ok(out);
        }
        for i in 0.. {
            let mut fs: sys::v4l2_frmsizeenum = zeroed();
            fs.index = i;
            fs.pixel_format = sys::V4L2_PIX_FMT_MJPEG;
            // SAFETY: valid fd and a correctly sized struct.
            match unsafe { sys::vidioc_enum_framesizes(self.raw(), &mut fs) } {
                Ok(_) => {}
                Err(Errno::EINVAL) => break,
                Err(e) => return Err(Error::ioctl("ENUM_FRAMESIZES", e)),
            }
            if fs.type_ != sys::V4L2_FRMSIZE_TYPE_DISCRETE {
                continue;
            }
            let (width, height) = (fs.u[0], fs.u[1]);
            for j in 0.. {
                let mut fi: sys::v4l2_frmivalenum = zeroed();
                fi.index = j;
                fi.pixel_format = sys::V4L2_PIX_FMT_MJPEG;
                fi.width = width;
                fi.height = height;
                // SAFETY: valid fd and a correctly sized struct.
                match unsafe { sys::vidioc_enum_frameintervals(self.raw(), &mut fi) } {
                    Ok(_) => {}
                    Err(Errno::EINVAL) => break,
                    Err(e) => return Err(Error::ioctl("ENUM_FRAMEINTERVALS", e)),
                }
                if fi.type_ != sys::V4L2_FRMIVAL_TYPE_DISCRETE || fi.u[0] == 0 {
                    continue;
                }
                let g = gcd(fi.u[0], fi.u[1]);
                let mode = Mode {
                    width,
                    height,
                    fps_n: fi.u[1] / g,
                    fps_d: fi.u[0] / g,
                };
                if !out.contains(&mode) {
                    out.push(mode);
                }
            }
        }
        out.sort_by(|a, b| {
            (u64::from(b.width) * u64::from(b.height))
                .cmp(&(u64::from(a.width) * u64::from(a.height)))
                .then(b.width.cmp(&a.width))
                .then(b.fps().total_cmp(&a.fps()))
        });
        Ok(out)
    }

    fn has_mjpeg(&self) -> Result<bool> {
        for i in 0.. {
            let mut f: sys::v4l2_fmtdesc = zeroed();
            f.index = i;
            f.type_ = sys::V4L2_BUF_TYPE_VIDEO_CAPTURE;
            // SAFETY: valid fd and a correctly sized struct.
            match unsafe { sys::vidioc_enum_fmt(self.raw(), &mut f) } {
                Ok(_) if f.pixelformat == sys::V4L2_PIX_FMT_MJPEG => return Ok(true),
                Ok(_) => {}
                Err(Errno::EINVAL) => break,
                Err(e) => return Err(Error::ioctl("ENUM_FMT", e)),
            }
        }
        Ok(false)
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}
