// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::battery::{History, Snapshot};

#[test]
fn battery_settings_and_status_bar_update_and_clear_stale_readings() {
    let (w, h) = (540, 1080);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    let empty = History::default();
    r.show("settings", "battery");
    r.set_battery(&Snapshot::default(), &empty, false);
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
    let mut history = History::default();
    for minute in 0..60 {
        history.observe(minute * 60, charging);
    }
    r.set_battery(&charging, &history, false);
    common::frame(&mut r, &mut buf, w, "charging battery");
    let status_end = w as usize * 37;
    assert!(buf[..status_end].iter().zip(&unknown).any(|(a, b)| a != b));
    assert!(
        buf[status_end..]
            .iter()
            .zip(&unknown[status_end..])
            .filter(|(a, b)| a != b)
            .count()
            > 100
    );

    r.set_battery(&Snapshot::default(), &empty, false);
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
        r.set_battery(&sample, &empty, false);
        common::frame(&mut r, &mut buf, w, "battery state indicator");
        let status = buf[..status_end].to_vec();
        assert!(
            frames.iter().all(|previous| previous != &status),
            "each charge/connection state needs a distinct status bar"
        );
        if charge == 1 || charge == 3 {
            assert!(
                !status.contains(&0xff22_b46a),
                "discharging and connected-idle must stay neutral"
            );
        }
        frames.push(status);
    }
    let invalid = Snapshot {
        valid: 0,
        ..charging
    };
    r.set_battery(&invalid, &empty, false);
    common::frame(&mut r, &mut buf, w, "invalid battery flags");
    assert_eq!(
        buf, unknown,
        "invalid stale fields must not show plug, lightning, or green"
    );
}
