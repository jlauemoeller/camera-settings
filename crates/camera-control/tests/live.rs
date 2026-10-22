//! Tests against a real, attached BRIO. Ignored by default:
//! `cargo test -p camera-control -- --ignored`

use camera_control::{Device, Kind, Mode, allowlist, find_camera};

fn open() -> (camera_control::CameraInfo, Device) {
    let info = find_camera().expect("no BRIO attached");
    let dev = Device::open(&info).expect("open main node");
    (info, dev)
}

#[test]
#[ignore = "needs a BRIO"]
fn discovery_and_read_only_queries() {
    let (info, dev) = open();
    println!("{info:#?}");
    assert!(!info.serial.is_empty());
    assert_eq!(info.unit_for_guid(allowlist::FOV.guid), Some(10));

    let controls = dev.controls().unwrap();
    for c in &controls {
        println!(
            "{:<28} {:?} {}..{} step {} def {} = {} inactive={}",
            c.id, c.kind, c.min, c.max, c.step, c.default, c.value, c.inactive
        );
    }
    let get = |id: &str| {
        controls
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("{id} missing"))
    };
    assert_eq!(
        controls.len(),
        allowlist::V4L2_CONTROLS.len() + 1,
        "all of §4.1 plus FOV"
    );
    let zoom = get("zoom");
    assert_eq!((zoom.min, zoom.max), (100, 500));
    let wb = get("white_balance_temperature");
    assert_eq!((wb.min, wb.max, wb.step), (2000, 7500, 10));
    let ae = get("exposure_auto");
    assert_eq!(ae.kind, Kind::Menu);
    let ae_values: Vec<_> = ae.options.as_ref().unwrap().iter().map(|o| o.value).collect();
    assert_eq!(ae_values, [1, 3]);
    let fov = get("fov");
    assert_eq!(fov.options.as_ref().unwrap().len(), 3);

    let modes = dev.modes().unwrap();
    println!("{modes:?}");
    assert!(modes.contains(&Mode::DEFAULT));
    if info.usb_speed_mbps >= 5000 {
        assert!(
            modes
                .iter()
                .any(|m| m.width == 3840 && m.height == 2160 && m.fps_n == 30 && m.fps_d == 1)
        );
    }
}

#[test]
#[ignore = "needs a BRIO"]
fn rejects_non_allowlisted() {
    let (_, dev) = open();
    assert!(matches!(dev.set("led", 1), Err(camera_control::Error::NotAllowed(_))));
    assert!(matches!(
        dev.set("zoom", 10_000),
        Err(camera_control::Error::OutOfRange { .. })
    ));
    assert!(matches!(
        dev.set("fov", 7),
        Err(camera_control::Error::OutOfRange { .. })
    ));
}

/// Writes allowlisted controls only and restores every original value afterwards.
#[test]
#[ignore = "needs a BRIO; writes (and restores) camera controls"]
fn write_and_restore() {
    let (_, dev) = open();
    let before = dev.controls().unwrap();
    let orig = |id: &str| before.iter().find(|c| c.id == id).unwrap().value;

    dev.set("brightness", 100).unwrap();
    assert_eq!(dev.control("brightness").unwrap().value, 100);

    dev.set("white_balance_auto", 0).unwrap();
    assert!(!dev.control("white_balance_temperature").unwrap().inactive);
    dev.set("white_balance_auto", 1).unwrap();
    assert!(
        dev.control("white_balance_temperature").unwrap().inactive,
        "INACTIVE follows auto WB"
    );

    let fov = orig("fov");
    dev.set("fov", if fov == 1 { 2 } else { 1 }).unwrap();
    assert_ne!(dev.control("fov").unwrap().value, fov);
    dev.set("fov", fov).unwrap();

    let mut restore: Vec<_> = before.iter().filter(|c| !c.read_only).collect();
    restore.sort_by_key(|c| allowlist::restore_order(c.id));
    for c in restore {
        let _ = dev.set(c.id, c.value);
    }
    let after = dev.controls().unwrap();
    for (a, b) in before.iter().zip(&after) {
        assert_eq!(a.value, b.value, "{} not restored", a.id);
    }
}

/// Run while another process streams the camera (e.g. `gst-launch-1.0 v4l2src ! fakesink`) with
/// `CAMERA_IN_USE=1`, and without it while nothing streams.
#[test]
#[ignore = "needs a BRIO"]
fn detects_stream_owner() {
    let (_, dev) = open();
    let expected = std::env::var_os("CAMERA_IN_USE").is_some();
    assert_eq!(dev.stream_in_use().unwrap(), expected);
    // Probing twice must not leave a claim behind.
    assert_eq!(dev.stream_in_use().unwrap(), expected);
}
