// SPDX-License-Identifier: GPL-3.0-only
//! NEOKIT.EXE: the neoui kit gallery on the device, and the smallest complete neoui app (docs `neoui-shell/kit.md`):
//! connect, show the UI, describe the page, then [`neoapp::App::run`]: input, resizes, Slint timers and drawing are
//! neoapp's; the app handles Back (through the kit's layers first) and what is its own. Its "Files" page shows how an
//! app uses Files' file picker and takes files from Open with (protocol 1.12: `open_file`, `result_file`, `arg_file`).
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::{Cell, RefCell};
use e32::dprintln;
use e32::fs::SharedFile;
use neoapp::open::{files, kit};
use neoapp::{App, AppEvent, Page};
use slint::ComponentHandle;

slint::include_modules!();

e32::main!(main);

fn main() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(e) => {
            dprintln!("kit: failed {}", e);
            e
        }
    }
}

/// The "Files" page's index in the header menu.
const FILES_PAGE: i32 = 9;
/// The "Notifications" page's.
const NOTIFY_PAGE: i32 = 10;

fn unix_ms() -> u64 {
    u64::try_from(e32::time::utc_now().0).unwrap_or(0) * 1000
}

/// TP3's sample track, playing from the start (no file behind it: the kit only shows the session and logs the
/// commands that come back).
fn sample_track() -> neoapp::media::Session {
    use neoapp::media::{Session, caps, state};
    Session {
        state: state::PLAYING,
        caps: caps::ALL,
        modes: 0,
        title: "Kit demo track".into(),
        artist: "NeoSymbian".into(),
        album: String::new(),
        art: 0,
        duration_ms: 180_000,
        position_ms: 0,
        at: unix_ms(),
    }
}

/// TP2a's sample cards (nt-shade): an ongoing call, three messages in one group, a copy in progress, a reminder.
fn samples() -> alloc::vec::Vec<neoapp::Notification> {
    use alloc::vec;
    use neoapp::Notification;
    use neoapp::card::{NOTIFY_ONGOING, action, style};
    let card = |id: u32, title: &str, text: &str| Notification {
        id,
        title: title.into(),
        text: text.into(),
        page: NOTIFY_PAGE as u16,
        ..Notification::default()
    };
    let chat = |id, who: &str, initials: &str, tint, text: &str| Notification {
        style: style::CONVERSATION,
        group: "chat".into(),
        initials: initials.into(),
        tint,
        actions: vec!["Reply".into(), "Mark read".into()],
        action_kinds: vec![action::REPLY, action::IN_PLACE],
        ..card(id, who, text)
    };
    vec![
        chat(1, "Grace Nakamura", "GN", 0xe5_484d, "Lunch tomorrow?"),
        chat(
            2,
            "Ben Okafor",
            "BO",
            0x3d_8bfd,
            "The build passed on the phone!",
        ),
        Notification {
            actions: vec!["Cancel".into()],
            action_kinds: vec![action::DANGER],
            style: style::PROGRESS,
            progress: Some(64),
            ..card(3, "Copying 48 photos", "768 MB of 1.2 GB")
        },
        card(4, "Lunch with Ben in 15 min", "13:00 · Harbour Café"),
        chat(
            5,
            "Ava Lindqvist",
            "AL",
            0x22_b46a,
            "Running late, see you at 7?",
        ),
        Notification {
            flags: NOTIFY_ONGOING,
            style: style::CALL,
            initials: "KT".into(),
            tint: 0x86_56e8,
            ..card(
                6,
                "Kit demo in progress",
                "An ongoing card: no swipe, no Clear all",
            )
        },
    ]
}
/// (TP5, TP6, TP4) The live demo's cards: a ringing call and a conversation to reply to inline.
const RING_CARD: u32 = 20;
const REPLY_CARD: u32 = 21;
/// Its channels: an Urgent one, so both banner.
const URGENT: u8 = 1;

/// The Urgent channel the live demo's cards post on (the kit is a policy owner, `n.kit`).
fn declare_urgent(app: &App) -> Result<(), i32> {
    app.channel(&neoapp::notification_policy::Channel {
        id: URGENT,
        importance: neoapp::notification_policy::importance::URGENT,
        name: "Live demo".into(),
        description: "The UI kit's ringing call and reply cards".into(),
    })
}

/// A ringing call as NEOPHONE posts it (TP5, O1): Urgent, Call style, sticky. With `SHELL_CALL_KEYS` the shell draws
/// Answer and Reject; the card's own actions are Silence and Reply with message.
fn ringing_card() -> neoapp::Notification {
    use neoapp::card::{NOTIFY_ONGOING, NOTIFY_STICKY, action, style};
    neoapp::Notification {
        id: RING_CARD,
        title: "Ava Lindqvist".into(),
        text: "+1 555 0142 · Mobile".into(),
        page: NOTIFY_PAGE as u16,
        style: style::CALL,
        flags: NOTIFY_STICKY | NOTIFY_ONGOING,
        channel: URGENT,
        initials: "AL".into(),
        tint: 0x22_b46a,
        actions: alloc::vec!["Silence".into(), "Reply with message".into()],
        action_kinds: alloc::vec![action::IN_PLACE, action::REPLY],
        ..neoapp::Notification::default()
    }
}

/// A conversation whose Reply the shell opens inline (TP4); the text comes back with REPLY_TEXT.
fn reply_card() -> neoapp::Notification {
    use neoapp::card::{action, style};
    neoapp::Notification {
        id: REPLY_CARD,
        title: "Ava Lindqvist".into(),
        text: "Yes. 7 at the usual place?\nRunning late, see you at 7?".into(),
        page: NOTIFY_PAGE as u16,
        style: style::CONVERSATION,
        channel: URGENT,
        initials: "AL".into(),
        tint: 0x22_b46a,
        actions: alloc::vec!["Reply".into(), "Mark read".into(), "Call".into()],
        action_kinds: alloc::vec![action::REPLY, action::IN_PLACE],
        ..neoapp::Notification::default()
    }
}

/// The live demo's activities (TP6): a timer (id 0) counting down 3 minutes, a call (id 1) counting up.
fn activities() -> [neoapp::activity::Activity; 2] {
    use neoapp::activity::{Activity, Trail, kind};
    let now = unix_ms();
    [
        Activity {
            id: 0,
            kind: kind::TIMER,
            page: NOTIFY_PAGE as u16,
            title: "Pasta".into(),
            text: "Timer".into(),
            trail: Trail::Until(now + 180_000),
            buttons: alloc::vec!["Pause".into(), "Cancel".into()],
            ..Activity::default()
        },
        Activity {
            id: 1,
            kind: kind::CALL,
            page: NOTIFY_PAGE as u16,
            title: "Ava Lindqvist".into(),
            text: "Mobile".into(),
            trail: Trail::Since(now),
            ..Activity::default()
        },
    ]
}

/// What the kit is waiting for from Files.
#[derive(Clone, Copy, PartialEq)]
enum Asked {
    Nothing,
    Pick,
    Save,
}

fn run() -> Result<(), i32> {
    let app = Rc::new(App::connect()?);
    let ui = KitApp::new().map_err(|_| -2)?;
    ui.global::<Theme>().set_dp(app.dp);
    let asked = Rc::new(Cell::new(Asked::Nothing));
    let demo = ui.global::<FileDemo>();
    let (a, q) = (app.clone(), asked.clone());
    demo.on_pick(move || {
        q.set(Asked::Pick);
        let _ = a.open(files::KEY, files::PICK, "");
    });
    let (a, q) = (app.clone(), asked.clone());
    demo.on_save(move || {
        q.set(Asked::Save);
        let _ = a.open(files::KEY, files::SAVE, "note.txt");
    });
    let cards = ui.global::<NotifyDemo>();
    let a = app.clone();
    cards.on_post(move || {
        let r = samples().iter().try_for_each(|n| a.notify(n));
        dprintln!("kit: sample cards posted ({:?})", r);
    });
    let a = app.clone();
    cards.on_clear(move || {
        dprintln!("kit: cards cleared ({:?})", a.clear_notify());
    });
    // (home.md Part 4) the kit's widget on home: a Text card that counts the sends
    let (a, sent) = (app.clone(), Rc::new(Cell::new(0u32)));
    cards.on_widget(move || {
        sent.set(sent.get() + 1);
        let r = a.widget(&neoapp::widget::Content {
            name: "demo".into(),
            page: NOTIFY_PAGE as u16,
            body: neoapp::widget::Body::Text {
                icon: 0,
                title: "Kit demo".into(),
                lines: alloc::vec![
                    format!("Sent {} times", sent.get()),
                    "From the UI kit".into()
                ],
            },
        });
        dprintln!("kit: home widget sent ({:?})", r);
    });
    // (TP5, TP6, TP4) the live demo
    let a = app.clone();
    cards.on_ring(move || {
        let keys = a
            .shell_version()
            .is_some_and(|v| v.has(neoapp::SHELL_CALL_KEYS));
        let r = declare_urgent(&a).and_then(|()| a.notify(&ringing_card()));
        dprintln!(
            "kit: ringing card posted ({:?}), shell call keys {}",
            r,
            keys
        );
    });
    let a = app.clone();
    cards.on_live(move || {
        let r = activities().iter().try_for_each(|x| a.activity(x));
        dprintln!("kit: activities posted ({:?})", r);
    });
    let a = app.clone();
    cards.on_reply(move || {
        let r = declare_urgent(&a).and_then(|()| a.notify(&reply_card()));
        dprintln!("kit: reply card posted ({:?})", r);
    });
    let a = app.clone();
    cards.on_end_live(move || {
        let r = (0..2).try_for_each(|id| a.end_activity(id));
        let _ = a.clear_notify();
        dprintln!("kit: live demo ended ({:?})", r);
    });
    // TP3: the sample track as the kit last posted it, while its session lasts
    let track: Rc<RefCell<Option<neoapp::media::Session>>> = Rc::default();
    let media = ui.global::<MediaDemo>();
    let (a, t) = (app.clone(), track.clone());
    media.on_play(move || {
        let s = sample_track();
        dprintln!("kit: media posted ({:?})", a.media(&s));
        *t.borrow_mut() = Some(s);
    });
    let (a, t) = (app.clone(), track.clone());
    media.on_end(move || {
        *t.borrow_mut() = None;
        dprintln!("kit: media ended ({:?})", a.end_media());
    });
    ui.show().map_err(|_| -2)?;
    let mut shown = Page::default();
    describe(&app, &ui, &mut shown)?;
    app.draw()?;
    app.run(|e| {
        match e {
            // Back closes the top kit layer (the ⋮ menu, a sheet); with none open the page's can_back is false and
            // Back leaves the app (the shell's)
            AppEvent::Back => {
                neoapp::kit_back!(ui);
            }
            AppEvent::Page { page } => ui.global::<GalleryState>().set_page(i32::from(page)),
            // the shade's answers to the sample cards (TP2a)
            AppEvent::Notified { id, action } => {
                dprintln!("kit: card {} opened ({})", id, action);
                ui.global::<GalleryState>().set_page(NOTIFY_PAGE);
                let status = format!("Card {id} opened (action {action}).");
                ui.global::<NotifyDemo>().set_status(status.into());
            }
            // (TP5) the banner's Answer: the ringing card goes, the call goes on in the island
            AppEvent::Notified {
                id: RING_CARD,
                action,
            } if action == neoapp::card::CALL_ANSWER || action == neoapp::card::CALL_REJECT => {
                dprintln!("kit: card {} opened ({})", RING_CARD, action);
                let _ = app.notify(&neoapp::Notification {
                    id: RING_CARD,
                    ..neoapp::Notification::default()
                });
                if action == neoapp::card::CALL_ANSWER {
                    dprintln!("kit: call activity ({:?})", app.activity(&activities()[1]));
                }
            }
            // (TP6) the island's taps and keys
            AppEvent::ActivityTap { id, i } => {
                dprintln!("kit: activity {} tap {}", id, i);
                if i == neoapp::activity::key::END || i == 2 {
                    let _ = app.end_activity(id);
                }
            }
            // (TP4) an inline reply: its text (never logged, only its length)
            AppEvent::NotifyReply { id } => match app.reply_text(id) {
                Ok(t) => {
                    dprintln!("kit: reply to card {}: {} bytes", id, t.len());
                    let status = format!("Replied to card {id}.");
                    ui.global::<NotifyDemo>().set_status(status.into());
                }
                Err(e) => dprintln!("kit: reply to card {} unreadable ({})", id, e),
            },
            AppEvent::NotifyGone { id, why } => {
                dprintln!("kit: card {} gone ({})", id, why);
                let how = if why == neoapp::card::GONE_SWIPED {
                    "swiped away"
                } else {
                    "cleared"
                };
                let status = format!("Card {id} {how}.");
                ui.global::<NotifyDemo>().set_status(status.into());
            }
            // TP3: the shade's or home's buttons (play and pause change the state the kit posts)
            AppEvent::Media { cmd, arg } => {
                use neoapp::media::{cmd as c, state};
                dprintln!("kit: media command {} {}", cmd, arg);
                let next = match cmd {
                    c::PAUSE => Some(state::PAUSED),
                    c::PLAY => Some(state::PLAYING),
                    _ => None,
                };
                if let Some(next) = next
                    && let Some(s) = track.borrow_mut().as_mut()
                {
                    let now = unix_ms();
                    s.position_ms = s.position_at(now);
                    s.at = now;
                    s.state = next;
                    let _ = app.media(s);
                }
                let status = format!("The shell sent command {cmd} ({arg}).");
                ui.global::<MediaDemo>().set_status(status.into());
            }
            // Open with: another app (Files) opened a file here
            AppEvent::Open { page } if page == kit::VIEW => {
                ui.global::<GalleryState>().set_page(FILES_PAGE);
                match app.arg_file() {
                    Some(f) => show(&ui, &f, "opened", "Opened here with Open with."),
                    None => ui
                        .global::<FileDemo>()
                        .set_status("Opened without a file.".into()),
                }
            }
            // back from Files: what it answered
            AppEvent::Visible(true) if asked.get() != Asked::Nothing => {
                let was = asked.replace(Asked::Nothing);
                match app.result_file() {
                    Some(f) if was == Asked::Save => {
                        let r = f.file().write(b"hello from kit\n");
                        let status = match r {
                            Ok(()) => "Saved through Files.",
                            Err(_) => "Files gave a file, but writing it failed.",
                        };
                        show(&ui, &f, "saved", status);
                    }
                    Some(f) => show(&ui, &f, "picked", "Picked in Files."),
                    None => ui
                        .global::<FileDemo>()
                        .set_status("Cancelled in Files.".into()),
                }
            }
            _ => {}
        }
        // the kit's controls: a pulse / a key's tone, as the profile says (D6)
        neoapp::kit_feedback!(ui, app);
        describe(&app, &ui, &mut shown)?;
        Ok(true)
    })
}

/// The page: Back stays in the app while a kit layer is open.
fn describe(app: &App, ui: &KitApp, shown: &mut Page) -> Result<(), i32> {
    let page = Page {
        title: "UI kit".into(),
        can_back: ui.global::<Kit>().get_layers() > 0,
        ..Page::default()
    };
    if page != *shown {
        app.page(&page)?;
        *shown = page;
    }
    Ok(())
}

/// Show a file the kit got: its full name, size and first line (at most 200 bytes, lossy UTF-8).
fn show(ui: &KitApp, f: &SharedFile, how: &str, status: &str) {
    let file = f.file();
    let name = file.full_name().unwrap_or_default();
    let size = file.size().unwrap_or(-1);
    let mut buf = [0u8; 200];
    let n = file.read_at(0, &mut buf).unwrap_or(0);
    let line = String::from_utf8_lossy(&buf[..n]);
    let line = line.lines().next().unwrap_or("");
    dprintln!(
        "kit: {} {} {} bytes, first line {:?}",
        how,
        name,
        size,
        line
    );
    let demo = ui.global::<FileDemo>();
    demo.set_status(status.into());
    demo.set_name(name.as_str().into());
    demo.set_size(format!("{size} bytes").into());
    demo.set_first_line(line.into());
}
