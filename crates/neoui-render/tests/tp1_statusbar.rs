// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::battery::Snapshot;
use neoui_proto::clock::face::{Face, LocalTime};
use neoui_proto::settings::Value;

/// TP1's status bar states that something backs (tp-statusbar), one under the other in `tp1-statusbar-pixel.png`:
/// idle, apps waiting (3 glyphs and +1), ring off with vibration and an alarm on, silent, charging with the battery
/// saver, airplane mode, No SIM, No service, roaming. Each one draws differently from idle.
#[test]
fn tp1_states() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    let bar = (25.0 * common::dp(w, h)) as usize * w as usize;
    let mon = LocalTime {
        year: 2026,
        month: 11,
        day: 9,
        weekday: 0,
        hour: 10,
        minute: 24,
    };
    let battery = |percent, usb| Snapshot {
        valid: 63,
        present: 1,
        percent,
        voltage_uv: 3_900_000,
        usb_present: usb,
        usb_online: usb,
        charge: if usb == 1 { 2 } else { 1 },
    };
    let idle = |r: &mut neoui_render::Renderer| {
        r.show("home", "");
        r.set_clock("10:24");
        r.set_now(&Face::new(&mon, true, ""));
        r.set_network(4, "Ucom AM", "LTE");
        r.set_network_state("", false);
        r.set_battery(&battery(72, 0));
        r.set_status_apps(&[]);
        for (k, v) in [
            ("sound.silent", false),
            ("sound.vibrate", true),
            ("battery.saver", false),
            ("radio.airplane", false),
        ] {
            r.apply_setting(k, &Value::Bool(v));
        }
    };
    type Step = fn(&mut neoui_render::Renderer);
    let states: [(&str, Step); 9] = [
        ("idle", |_| {}),
        ("apps waiting", |r| {
            r.set_status_apps(&["messages", "phone", "files", "clock"])
        }),
        ("ring off, vibrate, alarm", |r| {
            r.apply_setting("sound.silent", &Value::Bool(true));
            r.set_now(&Face::new(
                &LocalTime {
                    year: 2026,
                    month: 11,
                    day: 9,
                    weekday: 0,
                    hour: 10,
                    minute: 24,
                },
                true,
                "Tomorrow 07:15",
            ));
        }),
        ("silent", |r| {
            r.apply_setting("sound.silent", &Value::Bool(true));
            r.apply_setting("sound.vibrate", &Value::Bool(false));
        }),
        ("charging, saver", |r| {
            r.set_battery(&Snapshot {
                valid: 63,
                present: 1,
                percent: 48,
                voltage_uv: 3_900_000,
                usb_present: 1,
                usb_online: 1,
                charge: 2,
            });
            r.apply_setting("battery.saver", &Value::Bool(true));
        }),
        ("airplane", |r| {
            r.apply_setting("radio.airplane", &Value::Bool(true));
        }),
        ("no sim", |r| {
            r.set_network(0, "No service", "");
            r.set_network_state("No SIM", false);
        }),
        ("no service", |r| {
            r.set_network(0, "No service", "");
            r.set_network_state("No service", false);
        }),
        ("roaming", |r| r.set_network_state("", true)),
    ];
    let mut sheet = Vec::new();
    let mut seen: Vec<Vec<u32>> = Vec::new();
    for (name, step) in states {
        idle(&mut r);
        step(&mut r);
        common::frame(&mut r, &mut buf, w, name);
        let top = buf[..bar].to_vec();
        if let Some(first) = seen.first() {
            assert_ne!(&top, first, "{name} draws differently from idle");
        }
        seen.push(top.clone());
        sheet.extend_from_slice(&top);
    }
    let rows = u32::try_from(sheet.len()).unwrap() / w;
    common::write_png("tp1-statusbar-pixel.png", &sheet, w, rows);
}
