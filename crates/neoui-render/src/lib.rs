// SPDX-License-Identifier: Apache-2.0
//! Renders the neoui launcher with Slint's software renderer into a raw
//! XRGB8888 buffer. Platform-agnostic: the caller owns the buffer, the clock
//! and the input source.
#![no_std]

extern crate alloc;

mod home;
pub mod keyboard;
pub mod layer;
mod settings;
mod suggest;

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::time::Duration;

use neoui_proto::clock::face::Face;
use neoui_proto::ipc::KEYBOARD_MODES;
pub use neoui_proto::page::{Note, Notification, Page, Query};
use neoui_proto::settings::Value;
use neoui_proto::{Damage, InputEvent, Rect, ScrollPlan, Scrolling, TouchPhase};
use slint::platform::software_renderer::{
    LineBufferProvider, MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
    TargetPixel,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, PhysicalSize, PlatformError};

/// The most pages home has (`neoui_proto::home::MAX_PAGES`); [`Renderer::home_pages`] has how many it has now.
pub const HOME_PAGES: i32 = neoui_proto::home::MAX_PAGES as i32;

slint::include_modules!();

/// One installed app advertised by a launcher process. `icon` is the protocol's encoded ARGB icon (or empty).
#[derive(Clone, Debug)]
pub struct LauncherEntry {
    pub id: u32,
    pub caption: String,
    pub icon: Vec<u8>,
}

/// One XRGB8888 pixel, `0xFFRRGGBB` (alpha byte always 0xFF).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Xrgb8888(pub u32);

impl TargetPixel for Xrgb8888 {
    fn blend(&mut self, c: PremultipliedRgbaColor) {
        let inv = 255 - c.alpha as u32;
        let ch = |shift: u32, src: u8| ((self.0 >> shift & 0xFF) * inv / 255 + src as u32).min(255);
        self.0 = 0xFF00_0000 | ch(16, c.red) << 16 | ch(8, c.green) << 8 | ch(0, c.blue);
    }

    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self(0xFF00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32)
    }
}

struct NeoPlatform {
    window: Rc<MinimalSoftwareWindow>,
    now_ms: fn() -> u64,
    /// the shell's own text fields' clipboard (the edit bar's Copy and Paste), kept equal to the apps' by the server
    clipboard: Rc<keyboard::Clipboard>,
}

impl Platform for NeoPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        Duration::from_millis((self.now_ms)())
    }

    fn set_clipboard_text(&self, text: &str, clipboard: slint::platform::Clipboard) {
        if clipboard == slint::platform::Clipboard::DefaultClipboard {
            *self.clipboard.text.borrow_mut() = text.into();
            self.clipboard.copied.set(true);
        }
    }

    fn clipboard_text(&self, clipboard: slint::platform::Clipboard) -> Option<String> {
        (clipboard == slint::platform::Clipboard::DefaultClipboard)
            .then(|| self.clipboard.text.borrow().clone())
            .filter(|t| !t.is_empty())
    }
}

/// What the user did to the foreground app through the shell, for the app (`neoui_proto::ipc::AppEvent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    /// Back stays inside the app (its page said `can_back`).
    Back,
    /// ⋮ in the header the shell draws for the legacy bridge (it claimed `NAV_OPTIONS`).
    Options,
    /// A dialog button.
    Answer(u16),
    /// The toast's action ("Undo").
    NoteAction,
}

/// The live Back arrow (Navigation v2 rule 09): a `d` px bubble with a chevron, grey before the commit point and
/// accent past it. The pixel at (`x`, `y`) inside the bubble's square, or `None` outside the circle. The UI server
/// draws it over the frame (the app's surface is under it), so it is plain pixels, not Slint.
pub fn back_bubble_pixel(
    x: i32,
    y: i32,
    d: i32,
    committed: bool,
    pointing_left: bool,
) -> Option<u32> {
    let r = d / 2;
    let (dx, dy) = (2 * x + 1 - d, 2 * y + 1 - d); // twice the offset from the centre
    if dx * dx + dy * dy > d * d {
        return None;
    }
    // the chevron: two strokes meeting at the tip, a quarter of the bubble high each way
    let s = r / 2;
    let t = (d / 12).max(1); // the stroke's half width, x2
    let ax = if pointing_left { dx } else { -dx };
    // the strokes run from the tip (-s/2, 0) to (s/2, ±s), in doubled units
    let (tx, ex) = (-s, s);
    let on = |sign: i32| {
        let (vx, vy) = (ex - tx, 2 * s * sign);
        let (px, py) = (ax - tx, dy);
        let len2 = vx * vx + vy * vy;
        let k = (px * vx + py * vy).clamp(0, len2);
        let (cx, cy) = (px * len2 - k * vx, py * len2 - k * vy);
        // distance from the segment, compared without square roots
        cx as i64 * cx as i64 + cy as i64 * cy as i64 <= (t as i64 * 2 * len2 as i64).pow(2) / 4
    };
    Some(if on(1) || on(-1) {
        0xFFFF_FFFF
    } else if committed {
        0xFF3D_8BFD // Theme.accent
    } else {
        0xFF2A_2E35 // Theme.dialog-button
    })
}

/// Show touches (Settings SE3a): the pixel at (`x`, `y`) of a `d` px square around a finger, over `under` (what the
/// screen shows there): a white ring an eighth of `d` wide over a lightly whitened disc, or `None` outside the circle.
/// The UI server draws it straight to the screen, like the Back bubble.
#[must_use]
pub fn touch_ring_pixel(x: i32, y: i32, d: i32, under: u32) -> Option<u32> {
    let (dx, dy) = (2 * x + 1 - d, 2 * y + 1 - d); // twice the offset from the centre
    let r2 = dx * dx + dy * dy;
    if r2 > d * d {
        return None;
    }
    let inner = d - d / 4; // doubled: the ring is d/8 wide
    // white over `under` at `a`/256 per channel
    let mix = |a: u32| {
        let ch = |shift: u32| {
            let c = (under >> shift) & 0xFF;
            (c * (256 - a) + 0xFF * a) / 256
        };
        0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
    };
    Some(if r2 >= inner * inner {
        mix(208)
    } else {
        mix(64)
    })
}

/// At most this many app glyphs in the status bar; more apps show as "+n" (tp-statusbar).
pub const STATUS_APPS_MAX: usize = 3;

/// The status bar's app glyphs (tp-statusbar, TP1): one per app with cards in the shade, `keys` newest first (Phone
/// and Messages post their own cards since TP2b). Gives at most [`STATUS_APPS_MAX`] glyph ids (0 bell, 1 message,
/// 2 missed call, 3 folder, 4 alarm; `StatusBar` draws them) and how many more apps there are.
#[must_use]
pub fn status_apps(keys: &[&str]) -> (Vec<i32>, i32) {
    let mut apps: Vec<&str> = Vec::new();
    for k in keys.iter().copied() {
        if !apps.contains(&k) {
            apps.push(k);
        }
    }
    let glyph = |k: &str| match k {
        "messages" => 1,
        "phone" => 2,
        "files" => 3,
        "clock" | "dtsrv" => 4,
        _ => 0,
    };
    let shown = apps
        .iter()
        .take(STATUS_APPS_MAX)
        .map(|k| glyph(k))
        .collect();
    let more = apps.len().saturating_sub(STATUS_APPS_MAX);
    (shown, i32::try_from(more).unwrap_or(i32::MAX))
}

/// App `key`'s launcher badge: its own count, else for Phone and Messages MODEMD's (missed, unread); none when the
/// user turned its badge off (applied after the counts are combined, so the fallback can't bring it back).
fn badge_count(key: &str, own: i32, (missed, unread): (i32, i32), off: bool) -> i32 {
    if off {
        return 0;
    }
    match (key, own) {
        ("phone", 0) => missed,
        ("messages", 0) => unread,
        (_, n) => n,
    }
}

#[cfg(test)]
mod badge_tests {
    use super::badge_count;

    #[test]
    fn a_badge_turned_off_stays_off_with_the_fallback() {
        assert_eq!(badge_count("messages", 0, (2, 3), false), 3);
        assert_eq!(badge_count("messages", 0, (2, 3), true), 0);
        assert_eq!(badge_count("phone", 0, (2, 3), true), 0);
        assert_eq!(badge_count("phone", 4, (2, 3), false), 4);
        assert_eq!(badge_count("files", 1, (2, 3), true), 0);
    }
}

/// Card `n` of app `key` as the shade draws it (one on its own; the shade's rows set their section and group), its
/// time against `clock` (now unix ms, the UTC offset in s, 24-hour).
fn shade_note(key: &str, n: &Notification, clock: (u64, i32, bool)) -> ShadeNote {
    let (now, offset, h24) = clock;
    // two lines: the earlier one dim over the newest
    let (earlier, text) = n.text.rsplit_once('\n').unwrap_or(("", n.text.as_str()));
    let strings = |v: &[String]| {
        slint::ModelRc::from(Rc::new(slint::VecModel::from(
            v.iter()
                .map(|s| s.as_str().into())
                .collect::<Vec<slint::SharedString>>(),
        )))
    };
    ShadeNote {
        app_key: key.into(),
        app: neoui_proto::apps::find(key).map_or(key, |a| a.name).into(),
        group: n.group.as_str().into(),
        title: n.title.as_str().into(),
        earlier: earlier.into(),
        text: text.into(),
        actions: strings(&n.actions),
        kinds: slint::ModelRc::from(Rc::new(slint::VecModel::from(
            n.action_kinds
                .iter()
                .map(|k| i32::from(*k))
                .collect::<Vec<i32>>(),
        ))),
        initials: n.initials.as_str().into(),
        tint: slint::Color::from_argb_encoded(0xff00_0000 | n.tint),
        // an old record's own text; a 1.18 one's time as the shade writes it
        time: if n.when == 0 || !n.time.is_empty() {
            n.time.as_str().into()
        } else {
            neoui_proto::shade::when_text(n.when, now, offset, h24).into()
        },
        expanded: true,
        progress: n.progress.map_or(-1.0, |p| f32::from(p) / 100.0),
        section: "".into(),
        count: 1,
        open: false,
        member: false,
        ongoing: n.ongoing(),
    }
}

#[cfg(test)]
mod status_apps_tests {
    use super::status_apps;
    use alloc::vec;

    #[test]
    fn one_glyph_per_app_newest_first_then_plus_n() {
        assert_eq!(status_apps(&[]), (vec![], 0));
        // Messages twice (two conversations) is one app
        assert_eq!(
            status_apps(&["messages", "files", "messages"]),
            (vec![1, 3], 0)
        );
        assert_eq!(status_apps(&["browser", "phone"]), (vec![0, 2], 0));
        assert_eq!(
            status_apps(&["clock", "files", "store", "kit"]),
            (vec![4, 3, 0], 1)
        );
    }
}

#[cfg(test)]
mod touch_ring_tests {
    use super::touch_ring_pixel;

    #[test]
    fn a_ring_over_a_lighter_disc() {
        let d = 48;
        // the corner is outside, the edge is the ring, the centre is the disc
        assert_eq!(touch_ring_pixel(0, 0, d, 0), None);
        let edge = touch_ring_pixel(0, d / 2, d, 0).unwrap();
        let centre = touch_ring_pixel(d / 2, d / 2, d, 0).unwrap();
        assert!(edge & 0xFF > centre & 0xFF && centre & 0xFF > 0);
        // white stays white, opaque
        assert_eq!(
            touch_ring_pixel(d / 2, d / 2, d, 0xFFFF_FFFF),
            Some(0xFFFF_FFFF)
        );
    }
}

/// Owns the Slint platform and the launcher UI.
///
/// Slint's platform is a process-wide singleton (and with
/// `unsafe-single-threaded` it must only be used from one thread), so only one
/// `Renderer` can exist per process; a second `new` returns an error.
pub struct Renderer {
    window: Rc<MinimalSoftwareWindow>,
    ui: Shell,
    typed: Rc<RefCell<Vec<u32>>>,
    /// the keyboard's log and the shell fields' clipboard and chords (keyboard.rs)
    kb: keyboard::State,
    /// Presses of the system keyboard's keys since the last [`Renderer::take_key_presses`] (one per key, whatever it
    /// types and wherever it goes: Keypress vibration, SE3a).
    presses: Rc<core::cell::Cell<u32>>,
    nav: Rc<RefCell<Vec<Nav>>>,
    notes_opened: Rc<RefCell<Vec<(String, u32, u16)>>>,
    /// TP2a: the shade's rows → their cards (app key, id: a place in the shell's list can shift before the shell
    /// acts); the cards and the clock they were laid out with; the group shown open (app key, group); the cards
    /// swiped away; Clear all; a ▾ to lay out again.
    shade_rows: Rc<RefCell<Vec<(String, u32)>>>,
    shade_cards: Vec<(String, Notification)>,
    shade_clock: (u64, i32, bool),
    open_group: Rc<RefCell<Option<(String, String)>>>,
    notes_gone: Rc<RefCell<Vec<(String, u32)>>>,
    notes_cleared: Rc<Cell<bool>>,
    /// (TP4) the cards under Silent (a Min channel) and the card on the heads-up banner (app key, id)
    silent_cards: Vec<(String, u32)>,
    banner: Rc<RefCell<Option<(String, u32)>>>,
    relayout: Rc<Cell<bool>>,
    closed: Rc<RefCell<Vec<alloc::string::String>>>,
    power: Rc<RefCell<Vec<i32>>>,
    power_asked: Rc<Cell<bool>>,
    phone: Rc<RefCell<Vec<(String, String)>>>,
    /// keys of settings edited in the UI since the last `take_setting_edits` (`SystemSettings.edited`)
    edited: Rc<RefCell<Vec<String>>>,
    /// the apps' badge counts (BADGE) as last given, merged with what the shell knows itself
    app_badges: Vec<(String, u32)>,
    /// (TP4) apps whose badge the user turned off (their Badge or Allow switch): no count, the shell's fallback either
    badges_off: Vec<String>,
    /// The apps with cards in the shade, newest first (the status bar's glyphs, TP1).
    status_keys: Vec<String>,
    /// lists scrolled by shifting pixels (smooth-ui S2d), and how many frames did so
    scrolling: Scrolling,
    shifts: u32,
    /// (D10) home's saved layout and the editor's draft
    home: Rc<RefCell<home::State>>,
    /// (G3) the switcher's thumbnails, by app key
    thumbs: Vec<(String, slint::Image)>,
}

/// Slint's lines for a scrolled list's frame (smooth-ui S2d): the rows of `list` outside `draw` were moved into place
/// already ([`neoui_proto::shift_rows`]), so Slint skips them; everything else it draws as usual.
struct ScrollLines<'a> {
    px: &'a mut [Xrgb8888],
    stride: usize,
    list: Rect,
    draw: core::ops::Range<i32>,
}

impl LineBufferProvider for ScrollLines<'_> {
    type TargetPixel = Xrgb8888;

    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Xrgb8888]),
    ) {
        let (l, y) = (&self.list, line as i32);
        let moved = y >= l.y
            && y < l.y + l.h as i32
            && !self.draw.contains(&y)
            && range.start >= l.x as usize
            && range.end <= l.x as usize + l.w as usize;
        if !moved {
            render_fn(&mut self.px[line * self.stride..][range]);
        }
    }
}

impl Renderer {
    /// `now_ms` is a monotonic millisecond clock used for animations and timers.
    pub fn new(width: u32, height: u32, now_ms: fn() -> u64) -> Result<Self, PlatformError> {
        // The caller keeps one buffer and we redraw only what changed in it.
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        let clipboard: Rc<keyboard::Clipboard> = Rc::default();
        slint::platform::set_platform(Box::new(NeoPlatform {
            window: window.clone(),
            now_ms,
            clipboard: clipboard.clone(),
        }))
        .map_err(PlatformError::SetPlatformError)?;
        let ui = Shell::new()?;
        // before anything draws: bindings that call these keep what they first returned
        let kb = keyboard::wire(&ui, clipboard);
        window.set_size(PhysicalSize::new(width, height));
        ui.show()?;
        let typed: Rc<RefCell<Vec<u32>>> = Rc::default();
        let sink = typed.clone();
        let presses: Rc<core::cell::Cell<u32>> = Rc::default();
        let pressed = presses.clone();
        ui.on_typed(move |s| {
            pressed.set(pressed.get().saturating_add(1));
            sink.borrow_mut().extend(s.chars().map(u32::from));
        });
        let nav: Rc<RefCell<Vec<Nav>>> = Rc::default();
        let sink = nav.clone();
        ui.on_app_nav(move |kind, i| {
            let i = u16::try_from(i).unwrap_or(0);
            sink.borrow_mut().push(match kind.as_str() {
                "back" => Nav::Back,
                "options" => Nav::Options,
                "note" => Nav::NoteAction,
                _ => Nav::Answer(i),
            })
        });
        let notes_opened: Rc<RefCell<Vec<(String, u32, u16)>>> = Rc::default();
        let shade_rows: Rc<RefCell<Vec<(String, u32)>>> = Rc::default();
        let card = |rows: &Rc<RefCell<Vec<(String, u32)>>>, i: i32| {
            usize::try_from(i)
                .ok()
                .and_then(|i| rows.borrow().get(i).cloned())
        };
        let (sink, rows) = (notes_opened.clone(), shade_rows.clone());
        ui.on_note_opened(move |i, a| {
            if let Some((key, id)) = card(&rows, i) {
                sink.borrow_mut().push((key, id, a as u16));
            }
        });
        let notes_gone: Rc<RefCell<Vec<(String, u32)>>> = Rc::default();
        let (sink, rows) = (notes_gone.clone(), shade_rows.clone());
        ui.on_note_dismissed(move |i| {
            if let Some(c) = card(&rows, i) {
                sink.borrow_mut().push(c);
            }
        });
        let notes_cleared: Rc<Cell<bool>> = Rc::default();
        let sink = notes_cleared.clone();
        ui.on_notes_cleared(move || sink.set(true));
        // the banner's card acts as the shade's does
        let banner: Rc<RefCell<Option<(String, u32)>>> = Rc::default();
        let (sink, shown) = (notes_opened.clone(), banner.clone());
        ui.on_banner_opened(move |a| {
            if let Some((key, id)) = shown.borrow_mut().take() {
                sink.borrow_mut().push((key, id, a as u16));
            }
        });
        let open_group: Rc<RefCell<Option<(String, String)>>> = Rc::default();
        let relayout: Rc<Cell<bool>> = Rc::default();
        let (open, again, weak) = (open_group.clone(), relayout.clone(), ui.as_weak());
        ui.on_group_toggled(move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let Some(n) = usize::try_from(i)
                .ok()
                .and_then(|i| ui.get_notes().row_data(i))
            else {
                return;
            };
            let mut open = open.borrow_mut();
            let this = (
                String::from(n.app_key.as_str()),
                String::from(n.group.as_str()),
            );
            *open = if open.as_ref() == Some(&this) {
                None
            } else {
                Some(this)
            };
            again.set(true);
        });
        let closed: Rc<RefCell<Vec<alloc::string::String>>> = Rc::default();
        let sink = closed.clone();
        ui.on_close_app(move |k| sink.borrow_mut().push(k.as_str().into()));
        let sink = closed.clone();
        let shell = ui.as_weak();
        ui.on_close_all(move || {
            if let Some(ui) = shell.upgrade() {
                sink.borrow_mut()
                    .extend(ui.get_running().iter().map(|k| k.as_str().into()));
            }
        });
        let power: Rc<RefCell<Vec<i32>>> = Rc::default();
        let sink = power.clone();
        ui.on_power(move |a| sink.borrow_mut().push(a));
        let power_asked: Rc<Cell<bool>> = Rc::default();
        let sink = power_asked.clone();
        ui.on_power_asked(move || sink.set(true));
        let phone: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
        let sink = phone.clone();
        ui.on_phone_action(move |a, n| {
            sink.borrow_mut()
                .push((a.as_str().into(), n.as_str().into()))
        });
        let edited: Rc<RefCell<Vec<String>>> = Rc::default();
        let sink = edited.clone();
        ui.global::<SystemSettings>()
            .on_edited(move |k| sink.borrow_mut().push(k.as_str().into()));
        let home = home::wire(&ui, edited.clone());
        Ok(Self {
            window,
            ui,
            typed,
            kb,
            presses,
            nav,
            notes_opened,
            shade_rows,
            shade_cards: Vec::new(),
            shade_clock: (0, 0, true),
            open_group,
            notes_gone,
            notes_cleared,
            silent_cards: Vec::new(),
            banner,
            relayout,
            closed,
            power,
            power_asked,
            phone,
            edited,
            app_badges: Vec::new(),
            badges_off: Vec::new(),
            status_keys: Vec::new(),
            scrolling: Scrolling::default(),
            shifts: 0,
            home,
            thumbs: Vec::new(),
        })
    }

    /// Renders what changed since the last call into `buf` (XRGB8888, `stride_px`
    /// pixels per row, at least `stride_px * height` long) and returns the dirty region.
    /// `buf` must hold the previous frame (`RepaintBufferType::ReusedBuffer`).
    /// Jump straight to `screen` (an app key, "home" or "menu") and its sub-`view` ("" = the main view), closing
    /// any overlay. For snapshots and the host's `--screen`.
    pub fn show(&mut self, screen: &str, view: &str) {
        self.ui.set_screen(screen.into());
        self.ui.set_view(view.into());
        self.ui.set_shade_open(false);
    }

    /// Opens the shade (a swipe down on home, `shell.swipehome`).
    pub fn open_shade(&mut self) {
        self.ui.set_shade_open(true);
    }

    /// Shows app `screen` opened by app `from` (ipc `OPEN`): Back at its root returns to `from`.
    pub fn show_from(&mut self, screen: &str, from: &str) {
        if from != screen {
            self.ui.set_origin(from.into());
        }
        self.show(screen, "");
    }

    /// The shell's Back past the app (ipc `DONE`: the app's task ended): to where it was opened from, or home.
    pub fn leave_app(&mut self) {
        self.ui.invoke_back();
    }

    /// The current screen key: "home", "menu" or an app key such as "clock".
    pub fn screen(&self) -> alloc::string::String {
        self.ui.get_screen().as_str().into()
    }

    /// The current sub-page of the screen ("" = its root; Settings: "connectivity", "display", ...).
    pub fn view(&self) -> alloc::string::String {
        self.ui.get_view().as_str().into()
    }

    /// Where an app's content goes: the full-width band between the status bar and the toolbar (physical pixels).
    pub fn body(&self) -> Rect {
        let size = self.window.size();
        Rect::new(
            0,
            self.ui.get_body_y() as i32,
            size.width,
            self.ui.get_body_h() as u32,
        )
    }

    /// Where an app's surface shows: the body of an app screen (status bar above, toolbar below), whatever shows now.
    pub fn app_body(&self) -> Rect {
        let size = self.window.size();
        Rect::new(
            0,
            self.ui.get_body_y() as i32,
            size.width,
            self.ui.get_app_body_h() as u32,
        )
    }

    /// The toast (`NOTE`) while one shows, in physical pixels: it lies over the app's body, so the UI server copies it
    /// from the frame again after composing the app's surface.
    pub fn toast_rect(&self) -> Option<Rect> {
        (!self.ui.get_note_text().is_empty()).then(|| {
            Rect::new(
                self.ui.get_toast_x() as i32,
                self.ui.get_toast_y() as i32,
                // a pixel more each way: its edges may fall between pixels (it sits 12 dp clear of the screen's sides)
                self.ui.get_toast_w() as u32 + 1,
                self.ui.get_toast_h() as u32 + 1,
            )
        })
    }

    /// (TP4) The heads-up banner while it shows, in physical pixels: it lies over the app's body, so the UI server
    /// copies it from the frame again after composing the app's surface, and keeps touches in it from the app.
    pub fn banner_rect(&self) -> Option<Rect> {
        self.ui.get_banner_on().then(|| {
            Rect::new(
                self.ui.get_banner_x() as i32,
                self.ui.get_banner_y() as i32,
                // a pixel more each way: its edges may fall between pixels
                self.ui.get_banner_w() as u32 + 1,
                self.ui.get_banner_h() as u32 + 1,
            )
        })
    }

    /// The home screen's pages band, where its swipe works (physical pixels; smooth-ui S2c).
    pub fn home_strip(&self) -> Rect {
        Rect::new(
            0,
            self.ui.get_body_y() as i32,
            self.window.size().width,
            self.ui.get_home_strip_h() as u32,
        )
    }

    /// The home page on show (0..[`Renderer::home_pages`]).
    pub fn home_page(&self) -> i32 {
        self.ui.get_home_page()
    }

    /// Show home page `n` (at once with server transitions on).
    pub fn set_home_page(&mut self, n: i32) {
        self.ui.set_home_page(n.clamp(0, self.home_pages() - 1));
    }

    /// (D10) How many pages home has now (1..=5), or Lanes' three lanes.
    pub fn home_pages(&self) -> i32 {
        self.ui.get_home_page_count().max(1)
    }

    /// (D10) Home comes up in its editor (Settings' Edit home, `ipc::ACTION_EDIT_HOME`); asked again while editing,
    /// the draft stays.
    pub fn edit_home(&mut self) {
        self.show("home", "");
        self.ui
            .global::<HomeModel>()
            .invoke_op("enter".into(), 0, 0, 0, 0, "".into());
    }

    /// (D10) Is home's editor on? The UI server's page slide and pull to the shade stay off meanwhile.
    pub fn home_editing(&self) -> bool {
        self.home.borrow().editing()
    }

    /// (G3) The thumbnail of app `key` for Open apps: `w` × `h` XRGB8888 pixels (`layer::downsample`'s).
    pub fn set_thumb(&mut self, key: &str, w: usize, h: usize, px: &[u32]) {
        let (Ok(w32), Ok(h32)) = (u32::try_from(w), u32::try_from(h)) else {
            return;
        };
        if w == 0 || h == 0 || px.len() < w * h {
            return;
        }
        let mut buf = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(w32, h32);
        for (d, &p) in buf.make_mut_slice().iter_mut().zip(px) {
            *d = slint::Rgb8Pixel::new((p >> 16) as u8, (p >> 8) as u8, p as u8);
        }
        self.thumbs.retain(|(k, _)| k != key);
        self.thumbs.push((key.into(), slint::Image::from_rgb8(buf)));
        self.show_thumbs();
    }

    /// The thumbnails in the order of the running apps; those of apps no longer running go.
    fn show_thumbs(&mut self) {
        let running: Vec<String> = self
            .ui
            .get_running()
            .iter()
            .map(|k| k.as_str().into())
            .collect();
        self.thumbs.retain(|(k, _)| running.contains(k));
        let model: Vec<slint::Image> = running
            .iter()
            .map(|k| {
                self.thumbs
                    .iter()
                    .find(|(t, _)| t == k)
                    .map(|(_, i)| i.clone())
                    .unwrap_or_default()
            })
            .collect();
        self.ui
            .set_thumbs(slint::ModelRc::from(Rc::new(slint::VecModel::from(model))));
    }

    /// The UI server animates screen changes itself, from snapshots (smooth-ui S2): Slint switches screens at once.
    pub fn set_server_transitions(&mut self, on: bool) {
        self.ui.set_server_transitions(on);
    }

    /// Show the system keyboard over the bottom of the body in `mode` (`proto::KEYBOARD`: 1 text, 2 terminal,
    /// 3 number, 4 phone), or hide it (0).
    pub fn set_keyboard(&mut self, mode: u8) {
        self.ui.set_keyboard_mode(i32::from(mode));
    }

    /// The keyboard's height (physical pixels) in each `proto::KEYBOARD` mode: [0, text, terminal, number, phone].
    /// It changes with the language and the keyboard settings.
    pub fn keyboard_heights(&self) -> [u32; KEYBOARD_MODES] {
        [
            0,
            self.ui.get_keyboard_text_h() as u32,
            self.ui.get_keyboard_term_h() as u32,
            self.ui.get_keyboard_number_h() as u32,
            self.ui.get_keyboard_phone_h() as u32,
        ]
    }

    /// The keyboard's height (physical pixels) as shown: in the mode an app asked for, else in the one the focused
    /// shell field wants, else 0; 0 too while the user put it away ([`Self::keyboard_dismissed`]).
    pub fn keyboard_h(&self) -> u32 {
        let focus = self.ui.global::<KeyboardFocus>();
        if focus.get_dismissed() {
            return 0;
        }
        let mode = match self.ui.get_keyboard_mode() {
            0 if focus.get_open() => focus.get_mode(),
            m => m,
        };
        // an unknown mode is the text keyboard (as keyboard.slint draws it)
        let mode = if (0..KEYBOARD_MODES as i32).contains(&mode) {
            mode
        } else {
            1
        };
        self.keyboard_heights()[mode as usize]
    }

    /// (D11) The number pad is up (an app's or a shell field's): its taps may be PIN digits, so the shell logs no
    /// tap coordinates and draws no Show touches rings meanwhile.
    pub fn number_pad_up(&self) -> bool {
        let focus = self.ui.global::<KeyboardFocus>();
        let mode = match self.ui.get_keyboard_mode() {
            0 if focus.get_open() => focus.get_mode(),
            m => m,
        };
        mode == 3 && self.keyboard_h() > 0
    }

    /// The user put the keyboard away with the toolbar's keyboard button: the app in front gets its body back.
    pub fn keyboard_dismissed(&self) -> bool {
        self.ui.global::<KeyboardFocus>().get_dismissed()
    }

    /// Bring the keyboard back after the user put it away (an app asked for it again).
    pub fn clear_keyboard_dismissed(&mut self) {
        self.ui.global::<KeyboardFocus>().set_dismissed(false);
    }

    /// What the keyboard typed for an app since the last call, as Unicode code points (Backspace 8, Enter 13);
    /// what it types into a shell field goes there instead.
    pub fn take_typed(&mut self) -> Vec<u32> {
        core::mem::take(&mut *self.typed.borrow_mut())
    }

    /// How many keys of the system keyboard were pressed since the last call (for its vibration).
    pub fn take_key_presses(&mut self) -> u32 {
        self.presses.replace(0)
    }

    /// (D6) Whether one of the shell's own kit controls (a shade tile, a button, a row) acted since the last call: one
    /// pulse with vibrate on touch. The system keyboard isn't one (`keyboard.vibrate` is its own).
    pub fn take_accepted(&mut self) -> bool {
        let kit = self.ui.global::<Kit>();
        let (now, sent) = (kit.get_accepted(), kit.get_accepted_sent());
        kit.set_accepted_sent(now);
        now != sent
    }

    /// An app process is in front (the shell hides its own mock of the screen; the strip shows the app's page).
    pub fn set_app_front(&mut self, front: bool) {
        self.ui.set_app_front(front);
    }

    /// The foreground app's page (`neoui_proto::ipc::PAGE`): its name, whether Back stays inside it, full screen.
    pub fn set_app_page(&mut self, page: &Page) {
        let ui = &self.ui;
        ui.set_app_title(page.title.as_str().into());
        ui.set_app_back(page.can_back);
        ui.set_app_full(page.full);
    }

    /// The front app is the legacy bridge (no kit header): the shell draws a header over its surface, with ← while it
    /// keeps Back and ⋮ (`Nav::Options`) when it claimed `NAV_OPTIONS`.
    pub fn set_app_legacy(&mut self, legacy: bool, options: bool) {
        self.ui.set_app_legacy(legacy);
        self.ui.set_app_legacy_options(options);
    }

    /// The toast (`neoui_proto::ipc::NOTE`) to show above the bottom, or none.
    pub fn set_note(&mut self, note: Option<&neoui_proto::page::Note>) {
        let (text, action) = note.map_or(("", ""), |n| (n.text.as_str(), n.action.as_str()));
        self.ui.set_note_text(text.into());
        self.ui.set_note_action(action.into());
    }

    /// The apps' cards in the shade (`neoui_proto::ipc::NOTIFY`): (app key, card), newest first, as the shell keeps
    /// them; `now_ms` (unix), the zone's `offset_s` and `h24` write their times ("now", "2 min", "10:24"). The shade
    /// draws them by section and group (`neoui_proto::shade::layout`).
    pub fn set_notes(
        &mut self,
        notes: &[(&str, &Notification)],
        now_ms: u64,
        offset_s: i32,
        h24: bool,
    ) {
        self.shade_cards = notes
            .iter()
            .map(|(k, n)| ((*k).into(), (*n).clone()))
            .collect();
        self.shade_clock = (now_ms, offset_s, h24);
        // a group that went away isn't open any more
        let gone = self
            .open_group
            .borrow()
            .as_ref()
            .is_some_and(|(k, g)| !notes.iter().any(|(key, n)| key == k && n.group == *g));
        if gone {
            *self.open_group.borrow_mut() = None;
        }
        self.lay_out_notes();
    }

    fn lay_out_notes(&mut self) {
        let list: Vec<(&str, &Notification)> = self
            .shade_cards
            .iter()
            .map(|(k, n)| (k.as_str(), n))
            .collect();
        let open = self.open_group.borrow().clone();
        let silent = |i: usize| {
            let (k, n) = list[i];
            self.silent_cards
                .iter()
                .any(|(sk, id)| sk == k && *id == n.id)
        };
        let rows = neoui_proto::shade::layout_with(
            &list,
            open.as_ref().map(|(k, g)| (k.as_str(), g.as_str())),
            silent,
        );
        let clock = self.shade_clock;
        let model: Vec<ShadeNote> = rows
            .iter()
            .map(|r| {
                let (key, n) = list[r.index];
                ShadeNote {
                    section: if r.first { r.section.label() } else { "" }.into(),
                    count: i32::try_from(r.count).unwrap_or(i32::MAX),
                    open: open
                        .as_ref()
                        .is_some_and(|(k, g)| k == key && *g == n.group)
                        && r.count > 1,
                    member: r.member,
                    ..shade_note(key, n, clock)
                }
            })
            .collect();
        // the banner's card as it is now (gone from the shade: the banner goes too)
        let shown = self.banner.borrow().clone();
        match shown.and_then(|(k, id)| list.iter().find(|(lk, n)| *lk == k && n.id == id)) {
            Some((k, n)) => self.ui.set_banner(shade_note(k, n, clock)),
            None => {
                *self.banner.borrow_mut() = None;
                self.ui.set_banner_shown(false);
            }
        }
        *self.shade_rows.borrow_mut() = rows
            .iter()
            .map(|r| (String::from(list[r.index].0), list[r.index].1.id))
            .collect();
        self.ui
            .set_cards(i32::try_from(list.len()).unwrap_or(i32::MAX));
        self.ui
            .set_notes(slint::ModelRc::from(Rc::new(slint::VecModel::from(model))));
    }

    /// (TP4) The cards of a Min channel, (app key, id): the shade folds them under Silent. Before [`Self::set_notes`].
    pub fn set_silent_notes(&mut self, cards: &[(&str, u32)]) {
        self.silent_cards = cards.iter().map(|(k, id)| ((*k).into(), *id)).collect();
    }

    /// (TP4) The heads-up banner shows card (app key, id) of the shade's cards, or nothing; a tap or an action on it
    /// comes back with [`Self::take_notes_opened`], a swipe only hides it.
    pub fn set_banner(&mut self, card: Option<(&str, u32)>) {
        *self.banner.borrow_mut() = card.map(|(k, id)| (k.into(), id));
        self.ui.set_banner_shown(card.is_some());
        self.lay_out_notes();
    }

    /// The banner is on screen (the user didn't put it away).
    #[must_use]
    pub fn banner_shown(&self) -> bool {
        self.ui.get_banner_shown()
    }

    /// (TP4) Do not disturb's moon in the status bar.
    pub fn set_dnd(&mut self, on: bool) {
        self.ui.set_dnd(on);
    }

    /// The cards swiped away since the last call: (app key, id).
    pub fn take_notes_dismissed(&mut self) -> Vec<(String, u32)> {
        core::mem::take(&mut *self.notes_gone.borrow_mut())
    }

    /// Clear all was tapped since the last call.
    pub fn take_notes_cleared(&mut self) -> bool {
        self.notes_cleared.replace(false)
    }

    /// The shade's cards tapped since the last call: (app key, id, 0 for the card itself or `i + 1` for its action
    /// `i`).
    pub fn take_notes_opened(&mut self) -> Vec<(String, u32, u16)> {
        core::mem::take(&mut *self.notes_opened.borrow_mut())
    }

    /// The apps with cards in the shade, newest first (app keys, repeats allowed): the status bar's app glyphs.
    pub fn set_status_apps(&mut self, keys: &[&str]) {
        self.status_keys = keys.iter().map(|k| (*k).into()).collect();
        self.update_status_apps();
    }

    fn update_status_apps(&mut self) {
        let keys: Vec<&str> = self.status_keys.iter().map(String::as_str).collect();
        let (glyphs, more) = status_apps(&keys);
        self.ui
            .set_status_glyphs(slint::ModelRc::from(Rc::new(slint::VecModel::from(glyphs))));
        self.ui.set_status_more(more);
    }

    /// The launcher's badge counts (`neoui_proto::ipc::BADGE`) by app key; apps not named have none. Phone and
    /// Messages fall back to the missed calls and unread messages the shell knows from MODEMD while their app posts
    /// no count of its own.
    pub fn set_badges(&mut self, counts: &[(&str, u32)]) {
        self.app_badges = counts.iter().map(|(k, n)| ((*k).into(), *n)).collect();
        self.update_badges();
    }

    fn update_badges(&mut self) {
        let apps = self.ui.global::<Apps>();
        let own = |key: &str| {
            self.app_badges
                .iter()
                .filter(|(k, _)| k == key)
                .map(|(_, n)| *n as i32)
                .sum::<i32>()
        };
        let (missed, unread) = (self.ui.get_missed_calls(), self.ui.get_unread_messages());
        let model: Vec<i32> = apps
            .get_all()
            .iter()
            .map(|a| {
                let key = a.key.as_str();
                let off = self.badges_off.iter().any(|k| k == key);
                badge_count(key, own(key), (missed, unread), off)
            })
            .collect();
        apps.set_badges(slint::ModelRc::from(Rc::new(slint::VecModel::from(model))));
    }

    /// (TP4) The apps whose badge the user turned off: they show none, MODEMD's counts included.
    pub fn set_badges_off(&mut self, keys: &[&str]) {
        let keys: Vec<String> = keys.iter().map(|k| (*k).into()).collect();
        if keys != self.badges_off {
            self.badges_off = keys;
            self.update_badges();
        }
    }

    /// How long until a Slint timer is due (a long press, a toast's end), so the server wakes for it.
    pub fn next_timer(&self) -> Option<Duration> {
        slint::platform::duration_until_next_timer_update()
    }

    /// The foreground app's dialog (`neoui_proto::ipc::QUERY`), or none.
    pub fn set_app_query(&mut self, query: Option<&Query>) {
        let ui = &self.ui;
        ui.set_query_open(query.is_some());
        if let Some(q) = query {
            ui.set_query_text(q.text.as_str().into());
            let buttons: Vec<slint::SharedString> =
                q.buttons.iter().map(|b| b.as_str().into()).collect();
            ui.set_query_buttons(Rc::new(slint::VecModel::from(buttons)).into());
            ui.set_query_destructive(q.destructive);
        }
    }

    /// Show Buttons' key row over a full-screen page for a moment (a tap asked for it; rule 13), or hide it again.
    pub fn set_key_row_peek(&mut self, on: bool) {
        self.ui.set_key_row_peek(on);
    }

    /// Do what a finished edge gesture asks (docs `neoui-shell/navigation.md`, Navigation v2). Back runs the shell's Back, so an app
    /// whose page goes back inside it hears of it through [`Renderer::take_nav`]. Home from home opens the Apps menu.
    /// The last, previous and next apps come from the running apps (most recent first, [`Renderer::set_running`]).
    pub fn gesture(&mut self, g: neoui_proto::gesture::Gesture) {
        use neoui_proto::gesture::Gesture;
        let screen = self.screen();
        let running: Vec<alloc::string::String> = self
            .ui
            .get_running()
            .iter()
            .map(|k| k.as_str().into())
            .collect();
        let at = running.iter().position(|k| *k == screen);
        let switch_to = match g {
            Gesture::LastApp => running.iter().find(|k| **k != screen),
            // along the band: rightwards the older app, leftwards the newer one
            Gesture::PrevApp => running.get(at.map_or(0, |i| i + 1)),
            Gesture::NextApp => at
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| running.get(i)),
            _ => None,
        };
        // the home editor: going home or to the apps is its Done (D10)
        if screen == "home" && self.home_editing() && g != Gesture::Back {
            self.ui
                .global::<HomeModel>()
                .invoke_op("done".into(), 0, 0, 0, 0, "".into());
            return;
        }
        let lanes = self.ui.global::<SystemSettings>().get_home_style() == 1;
        match g {
            // Lanes (G5): Open apps is home's third lane
            Gesture::OpenApps if lanes => {
                self.show("home", "");
                self.ui.set_home_page(2);
            }
            // the UI server closes the app (G4); home comes up
            Gesture::Close => self.show("home", ""),
            Gesture::Back => self.ui.invoke_nav_back(),
            Gesture::OpenApps if screen != "switcher" => self.ui.invoke_open("switcher".into()),
            Gesture::Home if screen != "home" => self.show("home", ""),
            Gesture::OpenApps => {}
            Gesture::Home | Gesture::AppsMenu => self.show("menu", ""),
            Gesture::LastApp | Gesture::PrevApp | Gesture::NextApp => {
                if let Some(key) = switch_to {
                    self.ui.invoke_open(key.as_str().into());
                }
            }
        }
    }

    /// What the user did to the foreground app's page since the last call (the strip, the sheet, the dialog, Back).
    pub fn take_nav(&mut self) -> Vec<Nav> {
        core::mem::take(&mut *self.nav.borrow_mut())
    }

    /// The running apps' keys, most recent first, for the switcher (screen `switcher`).
    pub fn set_running(&mut self, keys: &[&str]) {
        let model: Vec<slint::SharedString> = keys.iter().map(|k| (*k).into()).collect();
        self.ui
            .set_running(slint::ModelRc::from(Rc::new(slint::VecModel::from(model))));
        self.show_thumbs();
    }

    /// Apps the switcher's × asked to close since the last call.
    pub fn take_closed(&mut self) -> Vec<alloc::string::String> {
        core::mem::take(&mut *self.closed.borrow_mut())
    }

    /// Is the shade or a dialog open over the app area? The UI server neither composes the app's surface over it nor
    /// gives it the touches meanwhile.
    pub fn overlay_open(&self) -> bool {
        self.ui.get_shade_open() || (self.ui.get_app_front() && self.ui.get_query_open())
    }

    /// Show or hide the Power menu (the UI server opens it when Power is held).
    pub fn set_power_menu(&mut self, open: bool) {
        self.ui.set_power_open(open);
        if open {
            self.ui.set_shade_open(false); // from the shade's ⏻: one change, nothing under them in between
        }
    }

    /// The sound profiles the Power menu offers (id, name) and the active one.
    pub fn set_power_profiles(&mut self, profiles: &[(u8, &str)], active: u8) {
        let model: Vec<PowerProfile> = profiles
            .iter()
            .map(|(id, name)| PowerProfile {
                id: i32::from(*id),
                name: (*name).into(),
            })
            .collect();
        self.ui
            .set_power_profiles(slint::ModelRc::new(slint::VecModel::from(model)));
        self.ui.set_power_profile(i32::from(active));
    }

    /// The shade's ⏻ was tapped since the last call (tp-shade): open the Power menu as for a held Power key.
    pub fn take_power_asked(&mut self) -> bool {
        self.power_asked.replace(false)
    }

    /// What the Power menu chose since the last call: 2 = power off, 0 = restart, -1 = dismissed, 100 + id = that
    /// sound profile.
    pub fn take_power(&mut self) -> Vec<i32> {
        core::mem::take(&mut *self.power.borrow_mut())
    }

    /// The registry keys of the settings the shell shows, for the UI server to fill at boot with
    /// [`Renderer::apply_setting`].
    pub fn shown_settings() -> impl Iterator<Item = &'static str> {
        settings::BINDINGS
            .iter()
            .map(|b| b.key)
            .chain(core::iter::once(home::KEY))
    }

    /// Shows a setting's value (from the UI server's store). False when the shell doesn't show `key` or `value` has
    /// another type than the key's.
    pub fn apply_setting(&mut self, key: &str, value: &Value) -> bool {
        if key == home::KEY {
            let Value::Bytes(b) = value else { return false };
            home::stored(&self.ui, &mut self.home.borrow_mut(), b);
            return true;
        }
        let global = self.ui.global::<SystemSettings>();
        settings::binding(key).is_some_and(|b| (b.write)(&global, value))
    }

    /// The UI's current value of a shown setting.
    pub fn setting(&self, key: &str) -> Option<Value> {
        if key == home::KEY {
            return Some(Value::Bytes(self.home.borrow().saved_bytes()));
        }
        let global = self.ui.global::<SystemSettings>();
        settings::binding(key).map(|b| (b.read)(&global))
    }

    /// The settings edited in the UI since the last call, each once, with their current values. Applying a value
    /// with [`Renderer::apply_setting`] reports it here too; storing it again changes nothing.
    pub fn take_setting_edits(&mut self) -> Vec<(&'static str, Value)> {
        let mut keys = core::mem::take(&mut *self.edited.borrow_mut());
        keys.sort_unstable();
        keys.dedup();
        let global = self.ui.global::<SystemSettings>();
        let mut edits: Vec<(&'static str, Value)> = keys
            .iter()
            .filter_map(|k| settings::binding(k))
            .map(|b| (b.key, (b.read)(&global)))
            .collect();
        // the home editor's Done (D10)
        if let Some(bytes) = self.home.borrow_mut().take_save() {
            edits.push((home::KEY, Value::Bytes(bytes)));
        }
        edits
    }

    /// The clock face the shell shows (the home clock widget): the time, the date and the next alarm.
    pub fn set_now(&mut self, face: &Face) {
        let now = self.ui.global::<Now>();
        // both are at most 59 / 23
        now.set_hours(face.hours as i32);
        now.set_minutes(face.minutes as i32);
        now.set_h24(face.h24);
        now.set_am_pm(face.am_pm.into());
        now.set_date(face.date.as_str().into());
        now.set_next_alarm(face.next_alarm.as_str().into());
    }

    /// The home agenda card's next event (`dtsrv.next`, DT7): when ("Today 13:00"), its title and its calendar's
    /// colour (0xRRGGBB). An empty title shows "No events".
    pub fn set_next_event(&mut self, when: &str, title: &str, rgb: u32) {
        let now = self.ui.global::<Now>();
        now.set_next_event_time(when.into());
        now.set_next_event_title(title.into());
        now.set_next_event_colour(slint::Color::from_argb_encoded(0xFF00_0000 | rgb));
    }

    /// The shade's date, in its format (`time.date_format`).
    pub fn set_date_text(&mut self, date: &str) {
        self.ui.global::<Now>().set_date_text(date.into());
    }

    /// The status bar's time text, e.g. "09:41".
    pub fn set_clock(&mut self, text: &str) {
        self.ui.set_clock(text.into());
    }

    /// The current link: Settings' "Current network" and its details, the status bar's Wi-Fi icon.
    pub fn set_wifi_link(&mut self, v: &neoui_proto::wifi::LinkView) {
        let details: alloc::vec::Vec<WifiDetail> = v
            .details
            .iter()
            .map(|(l, d)| WifiDetail {
                label: l.as_str().into(),
                value: d.as_str().into(),
            })
            .collect();
        self.ui.set_wifi_link(WifiLink {
            shown: v.shown,
            connected: v.connected,
            name: v.name.as_str().into(),
            subtitle: v.subtitle.as_str().into(),
            bars: i32::from(v.bars),
            band: v.band.as_str().into(),
            details: slint::ModelRc::new(slint::VecModel::from(details)),
            sign_in: v.sign_in,
        });
    }

    /// Shares the latest observation with the status bar (the Settings app reads the rest itself).
    pub fn set_battery(&mut self, sample: &neoui_proto::battery::Snapshot) {
        self.ui
            .set_battery_percent(sample.percent().map_or(-1, |p| p as i32));
        self.ui.set_battery_charging(sample.charging());
        self.ui.set_battery_connected(sample.usb_connected());
        self.ui.set_battery_full(sample.full_on_usb());
    }

    /// The status bar's network (telephony C5): `bars` 0..=4, or -1 without a modem; the operator's name; the
    /// technology ("LTE", "3G", "2G" or "").
    pub fn set_network(&mut self, bars: i32, operator: &str, rat: &str) {
        self.ui.set_bars(bars);
        self.ui.set_operator(operator.into());
        self.ui.set_rat(rat.into());
    }

    /// What the status bar says about the network instead of the operator's name (tp-statusbar): "" while
    /// registered, else "No SIM", "No service", "Recovering…"; `roaming` adds the R.
    pub fn set_network_state(&mut self, note: &str, roaming: bool) {
        self.ui.set_network_note(note.into());
        self.ui.set_roaming(roaming);
    }

    pub fn set_phone_feedback(&mut self, pending: bool, feedback: &str) {
        self.ui.set_phone_pending(pending);
        self.ui.set_phone_feedback(feedback.into());
    }

    pub fn set_sim(&mut self, pin: bool, blocked: bool, attempts: Option<u8>) {
        self.ui.set_sim_pin_required(pin);
        self.ui.set_sim_blocked(blocked);
        self.ui.set_pin_attempts(attempts.map_or(-1, i32::from));
    }

    /// The modem failed; `missed` new missed calls: Phone's badge while it posts no count of its own.
    pub fn set_phone_availability(&mut self, failed: bool, missed: u32) {
        self.ui.set_modem_failed(failed);
        self.ui.set_missed_calls(missed.min(i32::MAX as u32) as i32);
        self.update_badges();
    }

    /// Unread messages (C9): Messages' badge while it posts no count of its own.
    pub fn set_unread_messages(&mut self, count: u32) {
        self.ui
            .set_unread_messages(count.min(i32::MAX as u32) as i32);
        self.update_badges();
    }

    /// What the SIM prompt and the modem banner asked since the last call: "pin" + PIN, "sim-dismiss", "dial" +
    /// number (the prompt's emergency call), "retry".
    pub fn take_phone(&mut self) -> Vec<(String, String)> {
        core::mem::take(&mut *self.phone.borrow_mut())
    }

    pub fn render(&mut self, buf: &mut [u32], stride_px: usize) -> Damage {
        slint::platform::update_timers_and_animations();
        self.type_into_shell();
        // a group's ▾ (the callback can't reach the renderer): lay the cards out again
        if self.relayout.replace(false) {
            self.lay_out_notes();
        }
        // smooth-ui S2d: one list scrolled (`ui/components/scroll.slint`, its change handler ran above): move what it
        // showed, and Slint draws only the rows that came into view. Once the scrolling stops, one full redraw puts
        // right whatever the skipped rows missed (a highlight, a sub-pixel position).
        let sc = self.ui.global::<Scroll>();
        let (dy, lists) = (sc.get_dy(), sc.get_lists());
        sc.set_dy(0);
        sc.set_lists(0);
        let size = self.window.size();
        let list = Rect::new(
            sc.get_x() as i32,
            sc.get_y() as i32,
            sc.get_width() as u32,
            sc.get_height() as u32,
        );
        let screen = Rect::new(0, 0, size.width, size.height);
        let plan = self
            .scrolling
            .plan(buf, stride_px, screen, (dy, lists), list);
        let full = plan == ScrollPlan::Full;
        if full {
            self.window.request_redraw();
        } else if plan != ScrollPlan::Normal {
            self.shifts += 1;
        }
        // SAFETY: Xrgb8888 is repr(transparent) over u32, same size/alignment.
        let px = unsafe {
            core::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<Xrgb8888>(), buf.len())
        };
        let mut damage = Damage::new();
        self.window.draw_if_needed(|r| {
            if full {
                // a change of buffer type drops Slint's cache, so this render draws the whole window
                r.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
                r.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
            }
            let region = match plan {
                ScrollPlan::Shift { list, draw } => r.render_by_line(ScrollLines {
                    px,
                    stride: stride_px,
                    list,
                    draw,
                }),
                _ => r.render(px, stride_px),
            };
            for (o, s) in region.iter() {
                damage.add(Rect::new(o.x, o.y, s.width, s.height));
            }
        });
        damage
    }

    /// Feeds one input event to the UI. Touch positions are physical pixels
    /// (scale factor 1, so equal to Slint logical pixels).
    pub fn dispatch(&mut self, ev: InputEvent) {
        // shortcut: single pointer, all touch ids drive it; real multi-touch when Slint grows it.
        let event = match ev {
            InputEvent::Touch { x, y, phase, .. } => {
                let position = LogicalPosition::new(x as f32, y as f32);
                let button = PointerEventButton::Left;
                match phase {
                    TouchPhase::Down => {
                        self.ui.invoke_touch_down();
                        WindowEvent::PointerPressed { position, button }
                    }
                    TouchPhase::Move => WindowEvent::PointerMoved { position },
                    TouchPhase::Up => {
                        self.window
                            .dispatch_event(WindowEvent::PointerReleased { position, button });
                        // A lifted finger does not hover.
                        WindowEvent::PointerExited
                    }
                }
            }
            InputEvent::Key { code, down } => {
                let Some(text) = keyboard::key_text(code) else {
                    return;
                };
                if down {
                    WindowEvent::KeyPressed { text }
                } else {
                    WindowEvent::KeyReleased { text }
                }
            }
        };
        if let InputEvent::Touch {
            y,
            phase: TouchPhase::Down,
            ..
        } = ev
        {
            self.touch_outside_keyboard(y);
        }
        self.window.dispatch_event(event);
        self.type_into_shell();
    }

    /// Cancels the touch in progress: whatever it pressed lets go without a click (the UI server took the gesture
    /// over, e.g. its own home page slide).
    pub fn cancel_touch(&mut self) {
        self.window.dispatch_event(WindowEvent::PointerExited);
        self.stop_home_hold();
    }

    /// The touch in progress won't be a long press on home (Edit home · Wallpaper): the UI server takes it, or will
    /// (the pull down to the shade).
    pub fn stop_home_hold(&mut self) {
        self.ui.invoke_stop_home_hold();
    }

    /// Frames that scrolled a list by shifting its pixels since the last call (smooth-ui S2d).
    pub fn take_shifts(&mut self) -> u32 {
        core::mem::take(&mut self.shifts)
    }

    /// True while an animation runs, so the caller should keep rendering.
    pub fn has_active_animations(&self) -> bool {
        self.window.has_active_animations()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_blend() {
        assert_eq!(Xrgb8888::from_rgb(0x12, 0x34, 0x56).0, 0xFF12_3456);
        let mut p = Xrgb8888(0xFF00_00FF);
        // 50% premultiplied red over blue.
        p.blend(PremultipliedRgbaColor {
            red: 128,
            green: 0,
            blue: 0,
            alpha: 128,
        });
        assert_eq!(p.0, 0xFF80_007F);
        let mut q = Xrgb8888(0xFF00_00FF);
        q.blend(PremultipliedRgbaColor {
            red: 1,
            green: 2,
            blue: 3,
            alpha: 255,
        });
        assert_eq!(q.0, 0xFF01_0203);
    }
}

#[cfg(test)]
mod bubble_tests {
    use super::back_bubble_pixel;

    #[test]
    fn the_bubble_is_a_circle_with_a_chevron() {
        let d = 108; // 36 dp on the Pixel
        assert_eq!(
            back_bubble_pixel(0, 0, d, false, true),
            None,
            "a corner is outside"
        );
        assert_eq!(
            back_bubble_pixel(d / 2, 2, d, false, true),
            Some(0xFF2A_2E35)
        );
        assert_eq!(
            back_bubble_pixel(d / 2, 2, d, true, true),
            Some(0xFF3D_8BFD),
            "accent past the commit point"
        );
        // the tip of a left-pointing chevron is left of the centre, a right-pointing one's right of it
        let white = |x, left| back_bubble_pixel(x, d / 2, d, false, left) == Some(0xFFFF_FFFF);
        assert!(white(d / 2 - d / 8, true) && !white(d / 2 + d / 8, true));
        assert!(white(d / 2 + d / 8, false) && !white(d / 2 - d / 8, false));
    }
}
