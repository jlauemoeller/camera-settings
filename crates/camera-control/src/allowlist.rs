//! The only controls the app may touch (SPEC §4). Anything not listed here is rejected before
//! it reaches an ioctl. Ranges are *not* listed: they are read from the device.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    Framing,
    Exposure,
    Focus,
    Color,
}

/// A standard V4L2 control the app is allowed to read and write.
#[derive(Debug, Clone, Copy)]
pub struct V4l2Def {
    /// Stable id used by the UI and the config file.
    pub id: &'static str,
    pub cid: u32,
    pub label: &'static str,
    pub group: Group,
    /// Display hint for the frontend (`"100us"`, `"arcsec"`, `"kelvin"`, `"zoom"`).
    pub unit: Option<&'static str>,
    /// Restore order after (re)connect: lower first, so auto modes are applied before the
    /// manual values they gate, and zoom before pan/tilt.
    pub restore_order: u8,
}

const fn def(
    id: &'static str,
    cid: u32,
    label: &'static str,
    group: Group,
    unit: Option<&'static str>,
    restore_order: u8,
) -> V4l2Def {
    V4l2Def {
        id,
        cid,
        label,
        group,
        unit,
        restore_order,
    }
}

/// SPEC §4.1, in display order.
pub const V4L2_CONTROLS: &[V4l2Def] = &[
    def("zoom", 0x009a_090d, "Zoom", Group::Framing, Some("zoom"), 1),
    def("pan", 0x009a_0908, "Pan", Group::Framing, Some("arcsec"), 2),
    def("tilt", 0x009a_0909, "Tilt", Group::Framing, Some("arcsec"), 2),
    def("exposure_auto", 0x009a_0901, "Auto exposure", Group::Exposure, None, 0),
    def(
        "exposure_time",
        0x009a_0902,
        "Exposure time",
        Group::Exposure,
        Some("100us"),
        2,
    ),
    def("gain", 0x0098_0913, "Gain", Group::Exposure, None, 2),
    def(
        "exposure_dynamic_framerate",
        0x009a_0903,
        "Allow lower fps in low light",
        Group::Exposure,
        None,
        1,
    ),
    def(
        "backlight_compensation",
        0x0098_091c,
        "Backlight compensation",
        Group::Exposure,
        None,
        1,
    ),
    def(
        "power_line_frequency",
        0x0098_0918,
        "Power line frequency",
        Group::Exposure,
        None,
        1,
    ),
    def("focus_auto", 0x009a_090c, "Autofocus", Group::Focus, None, 0),
    def("focus", 0x009a_090a, "Focus", Group::Focus, None, 2),
    def(
        "white_balance_auto",
        0x0098_090c,
        "Auto white balance",
        Group::Color,
        None,
        0,
    ),
    def(
        "white_balance_temperature",
        0x0098_091a,
        "White balance",
        Group::Color,
        Some("kelvin"),
        2,
    ),
    def("brightness", 0x0098_0900, "Brightness", Group::Color, None, 2),
    def("contrast", 0x0098_0901, "Contrast", Group::Color, None, 2),
    def("saturation", 0x0098_0902, "Saturation", Group::Color, None, 2),
    def("sharpness", 0x0098_091b, "Sharpness", Group::Color, None, 2),
];

/// SPEC §4.2: the single extension-unit control the app may write.
pub struct XuDef {
    pub id: &'static str,
    pub label: &'static str,
    pub group: Group,
    pub guid: &'static str,
    pub selector: u8,
    pub size: u16,
    /// (raw value, label), in display order.
    pub options: &'static [(i64, &'static str)],
    pub restore_order: u8,
}

pub const FOV: XuDef = XuDef {
    id: "fov",
    label: "Field of view",
    group: Group::Framing,
    guid: "49e40215-f434-47fe-b158-0e885023e51b",
    selector: 5,
    size: 1,
    options: &[(0, "90°"), (1, "78°"), (2, "65°")],
    restore_order: 0,
};

pub fn v4l2_by_id(id: &str) -> Option<&'static V4l2Def> {
    V4L2_CONTROLS.iter().find(|d| d.id == id)
}

/// Restore priority for a control id (unknown ids sort last and are rejected later anyway).
pub fn restore_order(id: &str) -> u8 {
    if id == FOV.id {
        return FOV.restore_order;
    }
    v4l2_by_id(id).map_or(u8::MAX, |d| d.restore_order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_cids_are_unique() {
        for (i, a) in V4L2_CONTROLS.iter().enumerate() {
            for b in &V4L2_CONTROLS[i + 1..] {
                assert_ne!(a.id, b.id);
                assert_ne!(a.cid, b.cid);
            }
            assert_ne!(a.id, FOV.id);
        }
    }

    #[test]
    fn auto_modes_restore_before_manual_values() {
        assert!(restore_order("exposure_auto") < restore_order("exposure_time"));
        assert!(restore_order("focus_auto") < restore_order("focus"));
        assert!(restore_order("white_balance_auto") < restore_order("white_balance_temperature"));
        assert!(restore_order("zoom") < restore_order("pan"));
        assert_eq!(restore_order("led"), u8::MAX);
    }
}
