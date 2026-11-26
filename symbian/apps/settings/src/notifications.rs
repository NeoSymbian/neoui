// SPDX-License-Identifier: GPL-3.0-only
//! TP4 / D9: Panel & notifications, Notifications and one app's notifications (top-panel.md TP4, protocol 1.21). The
//! shell's policy (INFO 7: each app's switches and its channels with their effective importance) and the store's
//! `notify.*` keys fill the pages; the switches write `n.<app>` (`notification_policy::Policy`) and `notify.*`, and
//! the shell applies them (the same `effective()` it uses for the badge, shade, banner and sound). Read again with
//! every setting (`store::load`) and when the shell says INFO 7 moved on (event 27). A shell older than 1.21 answers
//! no INFO 7: the pages say so and list no apps. Card text never reaches these pages, search or the log.
use crate::Ctx;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use e32::dprintln;
use neoapp::App;
use neoui_proto::ipc as proto;
use neoui_proto::notification_policy::{
    self as policy, AppRecord, ChannelRecord, OVERRIDE_OFF, Policy, Record, flag, importance,
};
use neoui_proto::settings::{Value, keys};
use neoui_proto::settings_app::page::Page;
use neoui_proto::settings_app::row_model::{AppId, InstanceId};
use settings_ui::{ChannelRow, NotifyAppRow, SetState, SettingsApp};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

/// The banner's times on offer (seconds; `notify.bannert` is 1–30).
const BANNER_TIMES: [i32; 4] = [3, 5, 8, 10];

/// The shell's INFO 7: the app records and the channel records (`None`: the shell doesn't answer it).
fn records(app: &App) -> Option<(Vec<AppRecord>, Vec<ChannelRecord>)> {
    let (_, raw) = match app.settings_records(proto::INFO_CHANNELS) {
        Ok(r) => r,
        Err(e) => {
            dprintln!("settings: channels unavailable ({})", e);
            return None;
        }
    };
    let (mut apps, mut channels) = (Vec::new(), Vec::new());
    for b in &raw {
        match Record::decode(b) {
            Some(Record::App(a)) => apps.push(a),
            Some(Record::Channel(c)) => channels.push(c),
            None => {
                dprintln!("settings: a damaged channel record");
                return None;
            }
        }
    }
    Some((apps, channels))
}

/// App `owner`'s `n.<owner>` as stored (the defaults when there is none).
fn stored(app: &App, owner: &str) -> Policy {
    policy::key_of(owner)
        .and_then(|k| match app.setting_raw(k) {
            Ok(Value::Bytes(b)) => Policy::decode(&b),
            _ => None,
        })
        .unwrap_or_default()
}

/// Whose sound isn't the shell's: MODEMD rings calls and SMS, Clock its alarms (the shell plays them no chime).
fn sound_elsewhere(owner: &str) -> bool {
    matches!(owner, "phone" | "messages" | "dtsrv")
}

/// Everything the three pages and the root's line show, from the store and the shell.
pub fn refresh(app: &App, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    s.set_swipe_home(app.setting(keys::SHELL_SWIPEHOME).unwrap_or(true));
    let secs = app
        .setting(keys::NOTIFY_BANNERT)
        .unwrap_or(policy::BANNER_S);
    s.set_banner_time(secs);
    s.set_banner_text(format!("{secs} s").into());
    let times: Vec<SharedString> = BANNER_TIMES
        .iter()
        .map(|t| format!("{t} s").into())
        .collect();
    s.set_banner_times(ModelRc::new(VecModel::from(times)));
    s.set_banner_index(
        BANNER_TIMES
            .iter()
            .position(|t| *t == secs)
            .map_or(-1, |i| i as i32),
    );
    let dnd = app.setting(keys::NOTIFY_DND).unwrap_or(false);
    let until = match app.setting_raw(keys::NOTIFY_DNDTO.name()) {
        Ok(Value::Bytes(b)) => policy::dnd_until(&b).unwrap_or(0),
        _ => 0,
    };
    // what it really holds: the shell's banners and chimes; the modem still rings calls and SMS
    let until_text = (dnd && until != 0).then(|| {
        let (_, offset) = e32::time::utc_now();
        let m = (until + i64::from(offset)).rem_euclid(86_400) / 60;
        format!("Until {:02}:{:02}", m / 60, m % 60)
    });
    s.set_dnd(dnd);
    s.set_dnd_text(
        match (&until_text, dnd) {
            (Some(t), _) => format!("{t} · calls and SMS still ring"),
            (None, true) => String::from("On · calls and SMS still ring"),
            (None, false) => String::from("No banners or sounds from apps"),
        }
        .into(),
    );
    let got = records(app);
    s.set_notify_ok(got.is_some());
    let (apps, channels) = got.unwrap_or_default();
    let rows: Vec<NotifyAppRow> = apps
        .iter()
        .map(|a| NotifyAppRow {
            key: a.owner.as_str().into(),
            name: a.caption.as_str().into(),
            summary: policy::app_summary(a, &channels).into(),
            allowed: a.requested_flags & flag::ALLOW != 0,
        })
        .collect();
    let allowed = rows.iter().filter(|r| r.allowed).count();
    s.set_notify_root(
        match (&until_text, dnd) {
            (Some(t), _) => format!("Do not disturb · {}", t.to_lowercase()),
            (None, true) => String::from("Do not disturb"),
            (None, false) if apps.is_empty() => String::new(),
            (None, false) => format!("{allowed} of {} apps allowed", apps.len()),
        }
        .into(),
    );
    s.set_notify_apps(ModelRc::new(VecModel::from(rows)));
    let chans: Vec<ChannelRow> = channels
        .iter()
        .map(|c| ChannelRow {
            owner: c.owner.as_str().into(),
            id: i32::from(c.channel),
            name: c.name.as_str().into(),
            description: c.description.as_str().into(),
            value: policy::importance_name(c.effective).into(),
            level: i32::from(c.effective),
            general: c.channel == 0,
        })
        .collect();
    s.set_channels(ModelRc::new(VecModel::from(chans)));
    current(app, &s);
}

/// The app page's switches, from its stored policy.
fn current(app: &App, s: &SetState<'_>) {
    let owner = s.get_notify_current().key;
    if owner.is_empty() {
        return;
    }
    let p = stored(app, &owner);
    s.set_notify_allow(p.flags & flag::ALLOW != 0);
    s.set_notify_banner(p.flags & flag::BANNER != 0);
    s.set_notify_sound(p.flags & flag::SOUND != 0);
    s.set_notify_badge(p.flags & flag::BADGE != 0);
    s.set_notify_sound_ours(!sound_elsewhere(&owner));
    s.set_notify_sound_text(
        match owner.as_str() {
            "phone" | "messages" => "Calls and SMS tones come from the modem",
            "dtsrv" => "Clock rings its alarms itself",
            _ => "Message tone of the profile",
        }
        .into(),
    );
}

/// The app page is about `owner` (a policy owner); false for anything else.
fn select(app: &App, s: &SetState<'_>, owner: &str) -> bool {
    let Some(owner) = policy::OWNERS.iter().copied().find(|o| *o == owner) else {
        return false;
    };
    let row = s
        .get_notify_apps()
        .iter()
        .find(|a| a.key.as_str() == owner)
        .unwrap_or(NotifyAppRow {
            key: owner.into(),
            name: policy::caption(owner).into(),
            summary: SharedString::new(),
            allowed: true,
        });
    s.set_notify_current(row);
    current(app, s);
    true
}

/// A destination's app (Notifications' row, a channel's hit) before its page shows.
pub fn context(app: &App, s: &SetState<'_>, context: &Option<InstanceId>) {
    if let Some(
        InstanceId::App(AppId::Native(k))
        | InstanceId::Channel {
            owner: AppId::Native(k),
            ..
        },
    ) = context
    {
        select(app, s, k);
    }
}

/// The page opened: one app's page is titled by the app.
pub fn opened(s: &SetState<'_>, page: Page) {
    if page == Page::NotifyApp {
        s.set_title(s.get_notify_current().name);
    }
}

/// Write the current app's policy, changed by `f`.
fn change(app: &App, s: &SetState<'_>, f: impl FnOnce(&mut Policy)) {
    let owner = s.get_notify_current().key;
    let Some(key) = policy::key_of(&owner) else {
        return;
    };
    let mut p = stored(app, &owner);
    f(&mut p);
    let r = app.set_setting_raw(key, &Value::Bytes(p.encode()));
    dprintln!("settings: {} policy ({:?})", owner, r);
    if r.is_err() {
        let _ = app.note("Couldn't change the notifications", "");
    }
}

/// The pages' asks: `notify-dnd` 1/0, `notify-app` + an owner, `notify-flag` + "allow,1", `notify-importance` + a
/// choice (Urgent, Default, Low, Silent, Off) for the channel sheet's channel, `notify-bannert` + a choice.
pub fn act(ctx: &Ctx, ui: &SettingsApp, action: &str, value: &str) {
    let s = ui.global::<SetState>();
    let app = &ctx.app;
    match action {
        "notify-dnd" => {
            // on until turned off (a time comes with its picker); off forgets any end
            let r = app
                .set_setting_raw(keys::NOTIFY_DNDTO.name(), &Value::Bytes(Vec::new()))
                .and_then(|()| app.set_setting(keys::NOTIFY_DND, value == "1"));
            dprintln!("settings: do not disturb {} ({:?})", value, r);
        }
        "notify-app" => {
            if select(app, &s, value) {
                crate::row_model::go(ctx, Page::NotifyApp);
            }
        }
        // Notifications' per-app switch: Allow for that app
        "notify-allow" => {
            if let Some((owner, on)) = value.split_once(',')
                && select(app, &s, owner)
            {
                change(app, &s, |p| {
                    if on == "1" {
                        p.flags |= flag::ALLOW;
                    } else {
                        p.flags &= !flag::ALLOW;
                    }
                });
            }
        }
        "notify-flag" => {
            let Some((name, on)) = value.split_once(',') else {
                return;
            };
            let bit = match name {
                "allow" => flag::ALLOW,
                "banner" => flag::BANNER,
                "sound" => flag::SOUND,
                "badge" => flag::BADGE,
                _ => return,
            };
            change(app, &s, |p| {
                if on == "1" {
                    p.flags |= bit;
                } else {
                    p.flags &= !bit;
                }
            });
        }
        "notify-importance" => {
            let c = s.get_channel_current();
            let (Ok(channel), Ok(i)) = (u8::try_from(c.id), value.parse::<u8>()) else {
                return;
            };
            let want = match i {
                0 => importance::URGENT,
                1 => importance::DEFAULT,
                2 => importance::LOW,
                3 => importance::MIN,
                4 => OVERRIDE_OFF,
                _ => return,
            };
            // the app's own importance is no override: a later declaration may change it
            let declared = records(app).and_then(|(_, chans)| {
                chans
                    .into_iter()
                    .find(|x| x.owner == c.owner.as_str() && x.channel == channel)
                    .map(|x| x.declared)
            });
            let over = if declared == Some(want) { 0 } else { want };
            change(app, &s, |p| {
                p.set_override(channel, over);
            });
        }
        "notify-bannert" => {
            if let Some(t) = value
                .parse::<usize>()
                .ok()
                .and_then(|i| BANNER_TIMES.get(i))
            {
                let r = app.set_setting(keys::NOTIFY_BANNERT, *t);
                dprintln!("settings: banner {} s ({:?})", t, r);
            }
        }
        _ => {}
    }
}
