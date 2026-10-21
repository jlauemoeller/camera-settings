//! Find the camera through sysfs by VID:PID. `/dev/videoN` numbers change between replugs, so
//! the device node is always derived from the USB device, never remembered.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::descriptors;

pub const BRIO_VID: &str = "046d";
pub const BRIO_PID: &str = "085e";

const USB_DEVICES: &str = "/sys/bus/usb/devices";
const V4L_CLASS: &str = "/sys/class/video4linux";
const SOUND_CLASS: &str = "/sys/class/sound";

/// A camera found on the USB bus. Everything here comes from sysfs and needs no device access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    /// Canonical sysfs path of the USB device, e.g. `/sys/devices/.../usb4/4-1`.
    pub sysfs_path: PathBuf,
    /// Main capture node (`index` = 0), e.g. `/dev/video1`.
    pub node: PathBuf,
    pub model: String,
    pub serial: String,
    /// `bcdDevice` formatted as `major.minor`, e.g. `3.17`.
    pub firmware: String,
    /// USB link speed in Mbit/s (480 = USB 2, 5000+ = USB 3).
    pub usb_speed_mbps: u32,
    /// ALSA card number of the camera's own microphone, if it has one.
    pub alsa_card: Option<u32>,
    /// Extension-unit GUID → unit id, parsed from the USB descriptors.
    #[serde(skip)]
    pub extension_units: Vec<descriptors::ExtensionUnit>,
}

impl CameraInfo {
    pub fn unit_for_guid(&self, guid: &str) -> Option<u8> {
        self.extension_units
            .iter()
            .find(|u| u.guid.eq_ignore_ascii_case(guid))
            .map(|u| u.unit_id)
    }

    /// Still enumerated with the same node? Cheap check used before reusing a cached `CameraInfo`.
    pub fn is_present(&self) -> bool {
        self.sysfs_path.exists() && self.node.exists()
    }
}

fn read_attr(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_owned())
}

/// `bcdDevice` "0317" → "3.17".
fn format_bcd(bcd: &str) -> String {
    match u16::from_str_radix(bcd, 16) {
        Ok(v) => format!("{:x}.{:02x}", v >> 8, v & 0xff),
        Err(_) => bcd.to_owned(),
    }
}

/// The USB device directory owning a class device (`/sys/class/<cls>/<name>`): its `device`
/// link points at a USB interface (`4-1:1.0`) whose parent is the USB device (`4-1`).
fn owning_usb_device(class_dev: &Path) -> Option<PathBuf> {
    let iface = fs::canonicalize(class_dev.join("device")).ok()?;
    iface.parent().map(Path::to_path_buf)
}

/// Scan for the first BRIO. Returns `None` if none is attached or its main node isn't there yet
/// (udev may still be creating it).
pub fn find_camera() -> Option<CameraInfo> {
    find_camera_by_id(BRIO_VID, BRIO_PID)
}

pub fn find_camera_by_id(vid: &str, pid: &str) -> Option<CameraInfo> {
    let mut entries: Vec<_> = fs::read_dir(USB_DEVICES).ok()?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for dev in entries {
        if read_attr(&dev, "idVendor").as_deref() != Some(vid) || read_attr(&dev, "idProduct").as_deref() != Some(pid) {
            continue;
        }
        let Ok(sysfs_path) = fs::canonicalize(&dev) else {
            continue;
        };
        let Some(node) = main_video_node(&sysfs_path) else {
            continue;
        };
        let blob = fs::read(sysfs_path.join("descriptors")).unwrap_or_default();
        return Some(CameraInfo {
            node,
            model: read_attr(&sysfs_path, "product").unwrap_or_else(|| "Logitech BRIO".into()),
            serial: read_attr(&sysfs_path, "serial").unwrap_or_default(),
            firmware: read_attr(&sysfs_path, "bcdDevice")
                .map(|b| format_bcd(&b))
                .unwrap_or_default(),
            usb_speed_mbps: read_attr(&sysfs_path, "speed")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            alsa_card: alsa_card_for(&sysfs_path),
            extension_units: descriptors::extension_units(&blob),
            sysfs_path,
        });
    }
    None
}

/// The `/dev/videoN` with sysfs `index` 0 belonging to `usb_dev`.
fn main_video_node(usb_dev: &Path) -> Option<PathBuf> {
    let mut nodes: Vec<_> = fs::read_dir(V4L_CLASS).ok()?.flatten().map(|e| e.path()).collect();
    nodes.sort();
    nodes.into_iter().find_map(|class_dev| {
        if read_attr(&class_dev, "index").as_deref() != Some("0") {
            return None;
        }
        if owning_usb_device(&class_dev).as_deref() != Some(usb_dev) {
            return None;
        }
        let name = class_dev.file_name()?;
        let node = Path::new("/dev").join(name);
        node.exists().then_some(node)
    })
}

/// ALSA card whose sysfs parent is the same USB device as the camera.
fn alsa_card_for(usb_dev: &Path) -> Option<u32> {
    fs::read_dir(SOUND_CLASS).ok()?.flatten().find_map(|e| {
        let name = e.file_name().into_string().ok()?;
        let num: u32 = name.strip_prefix("card")?.parse().ok()?;
        (owning_usb_device(&e.path()).as_deref() == Some(usb_dev)).then_some(num)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcd_formatting() {
        assert_eq!(format_bcd("0317"), "3.17");
        assert_eq!(format_bcd("0017"), "0.17");
        assert_eq!(format_bcd("zz"), "zz");
    }
}
