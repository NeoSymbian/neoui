// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)]
mod common;

use neoui_proto::home::{Item, Layout};
use neoui_proto::settings::Value;
use neoui_render::Renderer;

/// The home editor (D10): Settings' Edit home enters it, × removes an item, Done saves once through the setting edits;
/// the store's answer settles it (the same bytes: saved; anything else: the draft comes back). Lanes (G5) shows its
/// three lanes, and the editor always edits the pages.
#[test]
fn home_editor() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let d = common::dp(w, h);
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "home");
    assert_eq!(r.home_pages(), 3, "the default layout's three pages");
    let _ = r.take_setting_edits();

    // Settings' Edit home: home in edit mode
    r.show("settings", "");
    r.edit_home();
    assert_eq!(r.screen(), "home");
    assert!(r.home_editing());
    common::frame(&mut r, &mut buf, w, "editing");
    common::write_png("home-edit-pixel.png", &buf, w, h);

    // × on the Clock widget (4 × 2 at the grid's top-left: × 18 dp in from its top-right corner, 6 dp outside it)
    let (x, y) = ((16.0 + 328.0 - 12.0) * d, (25.0 + 56.0 + 12.0) * d);
    common::tap(&mut r, x as i32, y as i32);
    common::frame(&mut r, &mut buf, w, "removed");
    assert!(
        r.take_setting_edits().is_empty(),
        "nothing saved before Done"
    );

    // Back is Done: one save, the empty pages trimmed
    r.gesture(neoui_proto::gesture::Gesture::Back);
    assert!(!r.home_editing());
    let edits = r.take_setting_edits();
    let [("home.layout", Value::Bytes(bytes))] = edits.as_slice() else {
        panic!("one save: {edits:?}");
    };
    let saved = Layout::decode(bytes).unwrap();
    assert_eq!(saved.pages.len(), 1);
    assert_eq!(saved.pages[0].items.len(), 3);
    assert!(!saved.pages[0].items.iter().any(|i| matches!(
        i,
        Item::Widget {
            widget: neoui_proto::home::Widget::Clock,
            ..
        }
    )));
    // the store took it: saved, one page
    assert!(r.apply_setting("home.layout", &Value::Bytes(bytes.clone())));
    common::frame(&mut r, &mut buf, w, "saved");
    assert_eq!(r.home_pages(), 1);
    assert_eq!(r.setting("home.layout"), Some(Value::Bytes(bytes.clone())));

    // a refused save: the store shows the old bytes again, the editor comes back with its draft
    r.edit_home();
    common::frame(&mut r, &mut buf, w, "editing again");
    common::tap(&mut r, x as i32, y as i32); // × on the first item (now Agenda)
    common::frame(&mut r, &mut buf, w, "removed again");
    r.gesture(neoui_proto::gesture::Gesture::Back);
    let edits = r.take_setting_edits();
    assert_eq!(edits.len(), 1, "{edits:?}");
    assert!(r.apply_setting("home.layout", &Value::Bytes(bytes.clone())));
    assert!(r.home_editing(), "the draft stays in the editor");
    common::frame(&mut r, &mut buf, w, "refused");
    common::write_png("home-edit-refused-pixel.png", &buf, w, h);
    // Home from the editor is its Done too
    r.gesture(neoui_proto::gesture::Gesture::Home);
    assert!(!r.home_editing());
    assert_eq!(r.screen(), "home");
    let edits = r.take_setting_edits();
    let [("home.layout", Value::Bytes(b2))] = edits.as_slice() else {
        panic!("{edits:?}");
    };
    assert_eq!(Layout::decode(b2).unwrap().pages[0].items.len(), 2);
    assert!(r.apply_setting("home.layout", &Value::Bytes(b2.clone())));

    // damaged bytes from the store: the default layout
    assert!(r.apply_setting("home.layout", &Value::Bytes(vec![9, 9, 9])));
    common::frame(&mut r, &mut buf, w, "damaged");
    assert_eq!(r.home_pages(), 3);

    // Lanes: three lanes; Open apps is the third
    assert!(r.apply_setting("home.style", &Value::Int(1)));
    r.set_running(&["clock", "files"]);
    common::frame(&mut r, &mut buf, w, "lanes");
    assert_eq!(r.home_pages(), 3);
    r.set_home_page(1);
    common::frame(&mut r, &mut buf, w, "lanes home");
    common::write_png("home-lanes-pixel.png", &buf, w, h);
    r.set_home_page(0);
    common::frame(&mut r, &mut buf, w, "lanes events");
    common::write_png("home-lanes-events-pixel.png", &buf, w, h);
    r.show("clock", "");
    r.gesture(neoui_proto::gesture::Gesture::OpenApps);
    assert_eq!((r.screen().as_str(), r.home_page()), ("home", 2));
    common::frame(&mut r, &mut buf, w, "lanes open apps");
    common::write_png("home-lanes-apps-pixel.png", &buf, w, h);
    // the editor edits the pages, whatever the style
    r.edit_home();
    common::frame(&mut r, &mut buf, w, "lanes editing");
    assert_eq!(r.home_pages(), 3);
    r.gesture(neoui_proto::gesture::Gesture::Back);
    let _: Vec<_> = r.take_setting_edits();
    let _ = Renderer::shown_settings();
}
