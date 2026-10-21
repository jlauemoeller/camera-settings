//! Raw kernel ABI: structs and ioctl numbers from `linux/videodev2.h` and `linux/uvcvideo.h`.
//!
//! Sizes are asserted at compile time against the values `gcc` reports for the system headers
//! on x86_64 (see the `const _` block at the bottom).

#![allow(non_camel_case_types)]

pub const V4L2_CTRL_FLAG_DISABLED: u32 = 0x0001;
pub const V4L2_CTRL_FLAG_READ_ONLY: u32 = 0x0004;
pub const V4L2_CTRL_FLAG_INACTIVE: u32 = 0x0010;
pub const V4L2_CTRL_FLAG_WRITE_ONLY: u32 = 0x0040;

pub const V4L2_CTRL_TYPE_INTEGER: u32 = 1;
pub const V4L2_CTRL_TYPE_BOOLEAN: u32 = 2;
pub const V4L2_CTRL_TYPE_MENU: u32 = 3;

pub const V4L2_CTRL_WHICH_CUR_VAL: u32 = 0;

pub const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
pub const V4L2_FRMSIZE_TYPE_DISCRETE: u32 = 1;
pub const V4L2_FRMIVAL_TYPE_DISCRETE: u32 = 1;
pub const V4L2_PIX_FMT_MJPEG: u32 = u32::from_le_bytes(*b"MJPG");

pub const UVC_GET_CUR: u8 = 0x81;
pub const UVC_SET_CUR: u8 = 0x01;
pub const UVC_GET_LEN: u8 = 0x85;
pub const UVC_GET_INFO: u8 = 0x86;
pub const UVC_GET_DEF: u8 = 0x87;

pub const UVC_INFO_GET: u8 = 0x01;
pub const UVC_INFO_SET: u8 = 0x02;

#[repr(C)]
pub struct v4l2_capability {
    pub driver: [u8; 16],
    pub card: [u8; 32],
    pub bus_info: [u8; 32],
    pub version: u32,
    pub capabilities: u32,
    pub device_caps: u32,
    pub reserved: [u32; 3],
}

#[repr(C)]
pub struct v4l2_query_ext_ctrl {
    pub id: u32,
    pub type_: u32,
    pub name: [u8; 32],
    pub minimum: i64,
    pub maximum: i64,
    pub step: u64,
    pub default_value: i64,
    pub flags: u32,
    pub elem_size: u32,
    pub elems: u32,
    pub nr_of_dims: u32,
    pub dims: [u32; 4],
    pub reserved: [u32; 32],
}

#[repr(C, packed)]
pub struct v4l2_querymenu {
    pub id: u32,
    pub index: u32,
    /// Union of `name[32]` and `s64 value`; only menu (name) controls are used here.
    pub name: [u8; 32],
    pub reserved: u32,
}

#[repr(C, packed)]
pub struct v4l2_ext_control {
    pub id: u32,
    pub size: u32,
    pub reserved2: u32,
    /// Union; for integer/bool/menu controls only the low 32 bits (`value`) are used.
    pub value64: i64,
}

#[repr(C)]
pub struct v4l2_ext_controls {
    pub which: u32,
    pub count: u32,
    pub error_idx: u32,
    pub request_fd: i32,
    pub reserved: u32,
    pub controls: *mut v4l2_ext_control,
}

#[repr(C)]
pub struct v4l2_fmtdesc {
    pub index: u32,
    pub type_: u32,
    pub flags: u32,
    pub description: [u8; 32],
    pub pixelformat: u32,
    pub mbus_code: u32,
    pub reserved: [u32; 3],
}

#[repr(C)]
pub struct v4l2_frmsizeenum {
    pub index: u32,
    pub pixel_format: u32,
    pub type_: u32,
    /// Union: discrete {width, height} or stepwise {min_w, max_w, step_w, min_h, max_h, step_h}.
    pub u: [u32; 6],
    pub reserved: [u32; 2],
}

#[repr(C)]
pub struct v4l2_frmivalenum {
    pub index: u32,
    pub pixel_format: u32,
    pub width: u32,
    pub height: u32,
    pub type_: u32,
    /// Union: discrete {numerator, denominator} or stepwise {min, max, step} fractions.
    pub u: [u32; 6],
    pub reserved: [u32; 2],
}

#[repr(C)]
pub struct uvc_xu_control_query {
    pub unit: u8,
    pub selector: u8,
    pub query: u8,
    pub size: u16,
    pub data: *mut u8,
}

nix::ioctl_read!(vidioc_querycap, b'V', 0, v4l2_capability);
nix::ioctl_readwrite!(vidioc_enum_fmt, b'V', 2, v4l2_fmtdesc);
nix::ioctl_readwrite!(vidioc_querymenu, b'V', 37, v4l2_querymenu);
nix::ioctl_readwrite!(vidioc_g_ext_ctrls, b'V', 71, v4l2_ext_controls);
nix::ioctl_readwrite!(vidioc_s_ext_ctrls, b'V', 72, v4l2_ext_controls);
nix::ioctl_readwrite!(vidioc_enum_framesizes, b'V', 74, v4l2_frmsizeenum);
nix::ioctl_readwrite!(vidioc_enum_frameintervals, b'V', 75, v4l2_frmivalenum);
nix::ioctl_readwrite!(vidioc_query_ext_ctrl, b'V', 103, v4l2_query_ext_ctrl);
nix::ioctl_readwrite!(uvcioc_ctrl_query, b'u', 0x21, uvc_xu_control_query);

const _: () = {
    assert!(size_of::<v4l2_capability>() == 104);
    assert!(size_of::<v4l2_query_ext_ctrl>() == 232);
    assert!(size_of::<v4l2_querymenu>() == 44);
    assert!(size_of::<v4l2_ext_control>() == 20);
    assert!(size_of::<v4l2_ext_controls>() == 32);
    assert!(size_of::<v4l2_fmtdesc>() == 64);
    assert!(size_of::<v4l2_frmsizeenum>() == 44);
    assert!(size_of::<v4l2_frmivalenum>() == 52);
    assert!(size_of::<uvc_xu_control_query>() == 16);
};

/// Zero-initialised value of a plain-old-data kernel struct.
pub fn zeroed<T>() -> T {
    // SAFETY: only used for the `repr(C)` structs above, for which all-zero bytes (including
    // null pointers) are a valid value.
    unsafe { std::mem::zeroed() }
}

/// NUL-terminated byte array → String.
pub fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
