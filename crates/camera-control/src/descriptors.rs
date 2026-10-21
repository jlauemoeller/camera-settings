//! Parser for the raw USB descriptor blob in `/sys/bus/usb/devices/<dev>/descriptors`.
//!
//! Only what the app needs: the UVC extension units (unit id + GUID) of the Video Control
//! interface, so vendor controls can be addressed by GUID instead of a hardcoded unit id.

const DT_INTERFACE: u8 = 0x04;
const DT_CS_INTERFACE: u8 = 0x24;
const CLASS_VIDEO: u8 = 0x0e;
const SUBCLASS_VIDEO_CONTROL: u8 = 0x01;
const VC_EXTENSION_UNIT: u8 = 0x06;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionUnit {
    pub unit_id: u8,
    /// Canonical lowercase GUID string, e.g. `49e40215-f434-47fe-b158-0e885023e51b`.
    pub guid: String,
}

/// Format a 16-byte USB GUID. The first three fields are stored little-endian.
pub fn format_guid(b: &[u8; 16]) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_le_bytes([b[4], b[5]]),
        u16::from_le_bytes([b[6], b[7]]),
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

/// All extension units declared in Video Control interfaces, in descriptor order.
pub fn extension_units(blob: &[u8]) -> Vec<ExtensionUnit> {
    let mut units = Vec::new();
    let mut in_video_control = false;
    let mut pos = 0;
    while pos + 2 <= blob.len() {
        let len = blob[pos] as usize;
        if len < 2 || pos + len > blob.len() {
            break;
        }
        let d = &blob[pos..pos + len];
        match d[1] {
            DT_INTERFACE if len >= 9 => {
                in_video_control = d[5] == CLASS_VIDEO && d[6] == SUBCLASS_VIDEO_CONTROL;
            }
            // bLength, bDescriptorType, bDescriptorSubtype, bUnitID, guidExtensionCode[16], ...
            DT_CS_INTERFACE if in_video_control && len >= 20 && d[2] == VC_EXTENSION_UNIT => {
                let guid: [u8; 16] = d[4..20].try_into().expect("slice of 16");
                units.push(ExtensionUnit {
                    unit_id: d[3],
                    guid: format_guid(&guid),
                });
            }
            _ => {}
        }
        pos += len;
    }
    units
}

/// Unit id of the extension unit with `guid`, if present.
pub fn find_unit(blob: &[u8], guid: &str) -> Option<u8> {
    extension_units(blob)
        .into_iter()
        .find(|u| u.guid.eq_ignore_ascii_case(guid))
        .map(|u| u.unit_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRIO: &[u8] = include_bytes!("../tests/fixtures/brio_usb3_descriptors.bin");

    #[test]
    fn resolves_brio_extension_units() {
        let units = extension_units(BRIO);
        let expect = [
            (14, "2c49d16a-32b8-4485-3ea8-643a152362f2"),
            (6, "23e49ed0-1178-4f31-ae52-d2fb8a8d3b48"),
            (8, "69678ee4-410f-40db-a850-7420d7d8240e"),
            (9, "1f5d4ca9-de11-4487-840d-50933c8ec8d1"),
            (10, "49e40215-f434-47fe-b158-0e885023e51b"),
            (11, "ffe52d21-8030-4e2c-82d9-f587d00540bd"),
            (12, "0f3f95dc-2632-4c4e-92c9-a04782f43bc8"),
            (21, "5a6d654c-7e35-4d4e-810d-069d15e0f79b"),
        ];
        for (id, guid) in expect {
            assert!(
                units.iter().any(|u| u.unit_id == id && u.guid == guid),
                "missing unit {id} {guid} in {units:?}"
            );
        }
        assert_eq!(units.len(), expect.len());
    }

    #[test]
    fn finds_fov_unit_by_guid() {
        assert_eq!(find_unit(BRIO, "49E40215-F434-47FE-B158-0E885023E51B"), Some(10));
        assert_eq!(find_unit(BRIO, "00000000-0000-0000-0000-000000000000"), None);
    }

    #[test]
    fn tolerates_truncated_input() {
        assert!(extension_units(&BRIO[..BRIO.len() / 3]).len() <= 8);
        assert!(extension_units(&[]).is_empty());
        assert!(extension_units(&[0, 0, 0]).is_empty());
    }
}
