//! Device-control layer for the Logitech BRIO: sysfs discovery, udev hotplug, and the
//! allowlisted V4L2 / UVC extension-unit controls from SPEC §4. No Tauri, no GStreamer.

pub mod allowlist;
pub mod descriptors;
pub mod device;
pub mod discovery;
pub mod hotplug;
mod sys;

pub use device::{Control, Device, Error, Kind, MenuOption, Mode, Result};
pub use discovery::{CameraInfo, find_camera};
pub use hotplug::{HotplugEvent, HotplugMonitor};
