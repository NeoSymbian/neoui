// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::battery::Snapshot;

#[test]
fn battery_status_bar_updates_and_clears_stale_readings() {
    let (w, h) = (540, 1080);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.set_battery(&Snapshot::default());
    common::frame(&mut r, &mut buf, w, "unknown battery");
    let unknown = buf.clone();

    let charging = Snapshot {
        valid: 63,
        present: 1,
        percent: 50,
        voltage_uv: 3_900_000,
        usb_present: 1,
        usb_online: 1,
        charge: 2,
    };
    r.set_battery(&charging);
    common::frame(&mut r, &mut buf, w, "charging battery");
    let status_end = w as usize * 37;
    assert!(buf[..status_end].iter().zip(&unknown).any(|(a, b)| a != b));

    r.set_battery(&Snapshot::default());
    common::frame(&mut r, &mut buf, w, "battery reading cleared");
    assert_eq!(
        buf, unknown,
        "failed readings must clear the percentage and charging state"
    );
    // Equal percentages isolate state glyphs: charging, full, plugged idle, and discharging.
    let mut frames = Vec::new();
    for (charge, connected) in [(2, true), (4, true), (3, true), (1, false)] {
        let sample = Snapshot {
            percent: 100,
            charge,
            usb_present: u32::from(connected),
            usb_online: u32::from(connected),
            ..charging
        };
        r.set_battery(&sample);
        common::frame(&mut r, &mut buf, w, "battery state indicator");
        let status = buf[..status_end].to_vec();
        assert!(
            frames.iter().all(|previous| previous != &status),
            "each charge/connection state needs a distinct status bar"
        );
        frames.push(status);
    }
    // the fill's colour is the level: red at or below 15 %, orange at or below 30 %, green above
    for (percent, colour) in [(10, 0xffe5_484d_u32), (25, 0xfff5_9e0b), (80, 0xff22_b46a)] {
        let sample = Snapshot {
            percent,
            charge: 1,
            usb_present: 0,
            usb_online: 0,
            ..charging
        };
        r.set_battery(&sample);
        common::frame(&mut r, &mut buf, w, "battery level colour");
        let status = &buf[..status_end];
        assert!(
            status.contains(&colour),
            "{percent} % fills with {colour:08x}"
        );
        for other in [0xffe5_484d_u32, 0xfff5_9e0b, 0xff22_b46a] {
            assert!(
                other == colour || !status.contains(&other),
                "{percent} % shows only its own level colour"
            );
        }
    }
    let invalid = Snapshot {
        valid: 0,
        ..charging
    };
    r.set_battery(&invalid);
    common::frame(&mut r, &mut buf, w, "invalid battery flags");
    assert_eq!(
        buf, unknown,
        "invalid stale fields must not show a bolt, a level or a colour"
    );
}
