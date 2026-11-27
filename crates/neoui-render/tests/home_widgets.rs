// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)]
mod common;

use neoui_proto::home::{Item, Layout, Size, Widget};
use neoui_proto::settings::Value;
use neoui_proto::widget::{Body, Content, Person};
use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Renderer;

fn differ(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b).filter(|(p, q)| p != q).count()
}

/// Pixels that differ inside the rectangle (x, y, w, h) of a `w`-wide frame.
fn differ_in(a: &[u32], b: &[u32], w: u32, (x, y, rw, rh): (u32, u32, u32, u32)) -> usize {
    (y..y + rh)
        .flat_map(|r| (x..x + rw).map(move |c| (r * w + c) as usize))
        .filter(|&i| a[i] != b[i])
        .count()
}

/// The layout the store holds now (the last save, taken as the store would).
fn saved(r: &mut Renderer) -> Layout {
    let edits = r.take_setting_edits();
    let [("home.layout", Value::Bytes(b))] = edits.as_slice() else {
        panic!("one save: {edits:?}");
    };
    assert!(r.apply_setting("home.layout", &Value::Bytes(b.clone())));
    Layout::decode(b).unwrap()
}

/// A long press at (x, y): down, a settled frame (the 500 ms timer fires), up.
fn hold(r: &mut Renderer, buf: &mut [u32], w: u32, x: i32, y: i32) {
    r.dispatch(InputEvent::Touch {
        id: 0,
        x,
        y,
        phase: TouchPhase::Down,
    });
    common::frame(r, buf, w, "held");
    r.dispatch(InputEvent::Touch {
        id: 0,
        x,
        y,
        phase: TouchPhase::Up,
    });
    common::frame(r, buf, w, "released");
}

/// Home Part 3's Apps menu long press and Part 4's 2 × 2 widgets, size choice and app widgets.
#[test]
fn home_part_3_and_4() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let d = common::dp(w, h);
    let px = |v: f32| (v * d) as i32;
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "home");
    let _ = r.take_setting_edits();
    // home's own long press (nav-home-longpress): Edit home · Wallpaper, held in the gap between two cards
    let home = buf.clone();
    hold(&mut r, &mut buf, w, 540, 1474);
    common::write_png("home-longpress-pixel.png", &buf, w, h);
    assert!(
        differ(&buf, &home) > 0,
        "the menu stays open after the release"
    );
    assert_eq!(r.screen(), "home", "a long press opens no app");
    // a tap outside it (in the same gap, right of the menu) closes it
    common::tap(&mut r, 1000, 1474);
    common::frame(&mut r, &mut buf, w, "menu closed");
    assert_eq!(differ(&buf, &home), 0, "a tap outside closes it");

    // --- the Apps menu: a long press on Clock (3rd column, 3rd row) opens the kit's Menu at the finger
    r.show("menu", "");
    common::frame(&mut r, &mut buf, w, "menu");
    let calm = buf.clone();
    let body_y = r.body().y;
    let (cx, cy) = (px(300.0), body_y + px(48.0 + 12.0 + 2.0 * 135.0 + 40.0));
    hold(&mut r, &mut buf, w, cx, cy);
    assert!(differ(&buf, &calm) > 0, "the menu opened");
    assert_eq!(r.screen(), "menu", "a long press opens nothing");
    common::write_png("sh-apps-menu-hold-pixel.png", &buf, w, h);
    // its one item, Add to home, under the finger (8 dp padding, a 48 dp row); the menu's left edge is clamped
    let item = (px(132.0 + 110.0), cy + px(8.0 + 24.0));
    common::tap(&mut r, item.0, item.1);
    // the toast, before its two seconds are up
    common::advance(300);
    r.render(&mut buf, w as usize);
    common::advance(300);
    r.render(&mut buf, w as usize);
    common::write_png("sh-apps-menu-added-pixel.png", &buf, w, h);
    common::frame(&mut r, &mut buf, w, "added");
    assert_eq!(r.screen(), "menu");
    let l = saved(&mut r);
    assert_eq!(
        l.pages[1].items,
        [Item::Shortcut("clock".into())],
        "page 0 is full: the first page with room"
    );
    // the toast goes after two seconds
    common::frame(&mut r, &mut buf, w, "toast gone");
    common::frame(&mut r, &mut buf, w, "toast gone");
    assert_eq!(differ(&buf, &calm), 0, "back as it was");

    // again: now Remove from home
    hold(&mut r, &mut buf, w, cx, cy);
    common::tap(&mut r, item.0, item.1);
    common::frame(&mut r, &mut buf, w, "removed");
    let l = saved(&mut r);
    assert!(l.pages.iter().all(|p| {
        p.items
            .iter()
            .all(|i| i.app() != "clock" || !matches!(i, Item::Shortcut(_)))
    }));

    // --- 2 × 2 widgets and an app's widget, greyed until its first content
    let mut l = Layout::default();
    l.pages[0].items = vec![
        Item::Widget {
            widget: Widget::Clock,
            size: Size::Square,
        },
        Item::Widget {
            widget: Widget::Agenda,
            size: Size::Square,
        },
        Item::Widget {
            widget: Widget::Favourites,
            size: Size::Square,
        },
        Item::Widget {
            widget: Widget::Media,
            size: Size::Square,
        },
    ];
    l.add_app_widget("kit", "demo", Size::Wide, 0).unwrap();
    l.pages[1].items.clear();
    l.add_app_widget("kit", "demo", Size::Square, 1).unwrap();
    l.add_app_widget("clock", "next", Size::Square, 1).unwrap();
    l.add_app_widget("kit", "demo", Size::Wide, 1).unwrap();
    assert!(r.apply_setting("home.layout", &Value::Bytes(l.encode().unwrap())));
    r.show("home", "");
    r.set_home_page(0);
    r.set_next_event("Today 13:00", "Lunch with Ben", 0x22_b46a);
    common::frame(&mut r, &mut buf, w, "2x2");
    common::write_png("home-2x2-pixel.png", &buf, w, h);

    r.set_home_page(1);
    common::frame(&mut r, &mut buf, w, "app widgets, no content");
    common::write_png("home-widgets-empty-pixel.png", &buf, w, h);
    let empty = buf.clone();
    // page 1: kit.demo 2 × 2 at the grid's top-left, clock.next beside it, kit.demo 4 × 1 under both
    let top = body_y as u32 + (22.0 * d) as u32;
    let row_h = {
        // HomeScreen: the strip under the dock and dots, 6 rows under the top margin
        let strip = r.home_strip();
        ((strip.h as f32 - 22.0 * d - 8.0 * d) / 6.0).max(56.0 * d)
    };
    let cell_w = (360.0 - 32.0) / 4.0 * d;
    let square = (
        (16.0 * d) as u32,
        top,
        (2.0 * cell_w) as u32,
        (2.0 * row_h) as u32,
    );
    let beside = (
        (16.0 * d + 2.0 * cell_w) as u32,
        top,
        (2.0 * cell_w) as u32,
        (2.0 * row_h) as u32,
    );
    let wide = (
        (16.0 * d) as u32,
        top + (2.0 * row_h) as u32,
        (4.0 * cell_w) as u32,
        row_h as u32,
    );

    let text = Content {
        name: "demo".into(),
        page: 0,
        body: Body::Text {
            icon: 0,
            title: "Kit demo".into(),
            lines: vec![
                "Sent by NEOKIT".into(),
                "Second line".into(),
                "Third".into(),
            ],
        },
    };
    r.set_app_widget("kit", "demo", Some(text));
    common::frame(&mut r, &mut buf, w, "text");
    common::write_png("home-widgets-text-pixel.png", &buf, w, h);
    assert!(
        differ_in(&buf, &empty, w, square) > 0,
        "the 2 × 2 shows the text"
    );
    assert!(
        differ_in(&buf, &empty, w, wide) > 0,
        "the 4 × 1 shows the text"
    );
    assert_eq!(
        differ_in(&buf, &empty, w, beside),
        0,
        "clock.next stays greyed: nothing came for it"
    );
    let shown = buf.clone();

    r.set_app_widget(
        "kit",
        "demo",
        Some(Content {
            name: "demo".into(),
            page: 0,
            body: Body::Count {
                icon: 0,
                value: 42,
                label: "unread".into(),
            },
        }),
    );
    common::frame(&mut r, &mut buf, w, "count");
    common::write_png("home-widgets-count-pixel.png", &buf, w, h);
    assert!(differ_in(&buf, &shown, w, square) > 0);

    let people = [
        "Ava Lindqvist",
        "Ben Okafor",
        "Dario Rossi",
        "Grace Nakamura",
        "Kit",
    ];
    let tints = [0x22_b46a, 0xe0_7a3a, 0xc0_4a30, 0xe5_484d, 0x86_56e8];
    r.set_app_widget(
        "kit",
        "demo",
        Some(Content {
            name: "demo".into(),
            page: 0,
            body: Body::People(
                people
                    .iter()
                    .zip(tints)
                    .map(|(n, tint)| Person {
                        initials: n
                            .split(' ')
                            .filter_map(|p| p.chars().next())
                            .take(2)
                            .collect(),
                        tint,
                        name: (*n).into(),
                    })
                    .collect(),
            ),
        }),
    );
    common::frame(&mut r, &mut buf, w, "people");
    common::write_png("home-widgets-people-pixel.png", &buf, w, h);
    // a tinted avatar shows in the 4 × 1
    let green = 0xFF22_B46A;
    let (x, y, rw, rh) = wide;
    assert!(
        (y..y + rh).any(|r| (x..x + rw).any(|c| buf[(r * w + c) as usize] == green)),
        "the first person's tint"
    );
    // none again: greyed
    r.set_app_widget("kit", "demo", None);
    common::frame(&mut r, &mut buf, w, "none again");
    assert_eq!(differ(&buf, &empty), 0);

    // --- the editor's Add sheet: each widget with its sizes; Agenda's 2 × 2 goes to the page shown
    r.edit_home();
    r.set_home_page(1);
    common::frame(&mut r, &mut buf, w, "editing");
    // + at the bar's left (8 dp padding, a 40 dp button)
    common::tap(&mut r, px(28.0), body_y + px(24.0));
    common::frame(&mut r, &mut buf, w, "add sheet");
    common::write_png("home-edit-add-pixel.png", &buf, w, h);
    // the rows from the sheet's bottom (12 dp padding, six 56 dp rows): Agenda is the second, its 2 × 2 the last chip
    let home_bottom = body_y + r.body().h as i32;
    let agenda_y = home_bottom - px(12.0 + 5.0 * 56.0 - 28.0);
    common::tap(&mut r, px(360.0 - 16.0 - 32.0), agenda_y);
    common::frame(&mut r, &mut buf, w, "added a 2x2");
    r.gesture(neoui_proto::gesture::Gesture::Back);
    let l = saved(&mut r);
    assert!(
        l.pages.iter().any(|p| p.items.contains(&Item::Widget {
            widget: Widget::Agenda,
            size: Size::Square
        })),
        "{l:?}"
    );
    let _ = Renderer::shown_settings();
}
