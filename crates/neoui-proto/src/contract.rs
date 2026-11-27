// SPDX-License-Identifier: Apache-2.0
//! What SYSSTART answers for protocol 1.22's opcodes and Settings actions before their shell side is built (W0a; docs
//! neoui-shell/protocol.md "Protocol 1.22"): the caller is checked first, then its record, and a request that would
//! be accepted answers `KErrNotSupported`. Each later stream replaces its part with the real handler and keeps the
//! checks: `WIDGET` is [`widget`] (home Part 4, W1-E), `MEDIA` is [`media`] (TP3), `TILE` is [`tile`] (TP7).

use crate::settings::access::Caller;
use crate::settings::kerr;
use crate::settings_app::{wallpaper, wire};
use crate::{activity, apps, ipc, media, tiles, widget};
use alloc::vec::Vec;

/// Whether `op` is one of 1.22's app → shell opcodes that publish for the caller and are not built yet (checked by
/// [`ingress`]). `MEDIA` (TP3), `TILE` (TP7) and `WIDGET` are built: the shell checks them with [`media`], [`tile`]
/// and [`widget`] and keeps them.
#[must_use]
pub fn is_publisher_opcode(op: i32) -> bool {
    matches!(op, ipc::REPLY_TEXT | ipc::ACTIVITY)
}

/// (TP7) A `TILE` from app `key` whose message carries secure ID `sid`, checked as [`ingress`] checks the others:
/// `KErrPermissionDenied` unless `key` has a catalogue SID and `sid` is it, then `KErrArgument` for a malformed offer
/// (`read` refuses one past [`tiles::OFFER_MAX`] with `KErrTooBig`).
///
/// # Errors
/// As above.
pub fn tile(
    key: &str,
    sid: u32,
    read: impl FnOnce(usize) -> Result<Vec<u8>, i32>,
) -> Result<tiles::Offer, i32> {
    if apps::sid(key) != Some(sid) {
        return Err(kerr::PERMISSION_DENIED);
    }
    tiles::Offer::decode(&read(tiles::OFFER_MAX)?).ok_or(kerr::ARGUMENT)
}

/// The answer to publisher opcode `op` from app `key` whose message carries secure ID `sid`. `read(max)` reads the
/// record (slot 0), refusing one over `max` bytes; it is called only once the caller is known.
///
/// `KErrPermissionDenied` unless `key` has a catalogue SID and `sid` is it (the legacy bridge and the kit's shared
/// SID publish nothing); then `KErrArgument` for a malformed record (an ACTIVITY of kind Media too: that one is the
/// shell's); else `KErrNotSupported`.
pub fn ingress(
    op: i32,
    key: &str,
    sid: u32,
    read: impl FnOnce(usize) -> Result<Vec<u8>, i32>,
) -> i32 {
    if apps::sid(key) != Some(sid) {
        return kerr::PERMISSION_DENIED;
    }
    let ok = |valid: bool| if valid { Ok(()) } else { Err(kerr::ARGUMENT) };
    let r = match op {
        ipc::ACTIVITY => read(activity::ACTIVITY_MAX).and_then(|b| {
            ok(activity::Activity::decode(&b).is_some_and(|a| a.kind != activity::kind::MEDIA))
        }),
        // REPLY_TEXT reads nothing: no reply is pending before TP4's inline reply
        _ => Ok(()),
    };
    r.err().unwrap_or(kerr::NOT_SUPPORTED)
}

/// `WIDGET` (46) from app `key` whose message carries secure ID `sid`: the content to keep, checked as for
/// [`ingress`] (the caller first, `read` only then): `KErrPermissionDenied` unless `key` has a catalogue SID and `sid`
/// is it, so only the app's own EXE fills its widgets; `KErrTooBig` / `KErrArgument` for a long or malformed record,
/// `KErrNotFound` for a name not in the app's catalogue, `KErrArgument` for another template.
///
/// # Errors
/// As above.
pub fn widget(
    key: &str,
    sid: u32,
    read: impl FnOnce(usize) -> Result<Vec<u8>, i32>,
) -> Result<widget::Content, i32> {
    if apps::sid(key) != Some(sid) {
        return Err(kerr::PERMISSION_DENIED);
    }
    let c = widget::Content::decode(&read(widget::WIDGET_MAX)?).ok_or(kerr::ARGUMENT)?;
    c.fits(key).map_err(|template| {
        if template {
            kerr::ARGUMENT
        } else {
            kerr::NOT_FOUND
        }
    })?;
    Ok(c)
}

/// `MEDIA` (TP3): the caller as for every publisher (`KErrPermissionDenied` without its catalogue SID), then the
/// record: `Ok(None)` for the empty one (the caller's session ends), `KErrArgument` for a malformed one.
pub fn media(
    key: &str,
    sid: u32,
    read: impl FnOnce(usize) -> Result<Vec<u8>, i32>,
) -> Result<Option<media::Session>, i32> {
    if apps::sid(key) != Some(sid) {
        return Err(kerr::PERMISSION_DENIED);
    }
    let b = read(media::MEDIA_MAX)?;
    if b.is_empty() {
        return Ok(None);
    }
    media::Session::decode(&b).map(Some).ok_or(kerr::ARGUMENT)
}

/// The longest `SETTINGS_ACTION` request the checks below read.
pub const ACTION_REQUEST_MAX: usize = wire::WallpaperRequest::LEN;

/// (TP9) `SETTINGS_ACTION` 14 `LOCK_DIALOG`'s checks before SYSSTART opens its PIN dialog: `KErrPermissionDenied`
/// unless the caller is Settings (nothing is read before), `KErrArgument` for a malformed request; else the mode
/// (`wire::lock_dialog`). The digits never cross IPC: the dialog is the shell's own.
///
/// # Errors
/// As above.
pub fn lock_dialog_request(
    caller: Caller<'_>,
    read: impl FnOnce(usize) -> Result<Vec<u8>, i32>,
) -> Result<u8, i32> {
    if caller != Caller::Settings {
        return Err(kerr::PERMISSION_DENIED);
    }
    read(ACTION_REQUEST_MAX)
        .ok()
        .and_then(|b| wire::lock_dialog_request(&b))
        .ok_or(kerr::ARGUMENT)
}

/// (D7) `SETTINGS_ACTION` 8 `INSTALL_WALLPAPER`'s checks before the shell takes the file: `KErrPermissionDenied`
/// unless the caller is Settings (nothing is read before), `KErrArgument` for a malformed request,
/// `KErrNotSupported` for a size the shell doesn't draw (only [`wallpaper::REQUEST`]); else the request.
///
/// # Errors
/// As above.
pub fn wallpaper_request(
    caller: Caller<'_>,
    read: impl FnOnce(usize) -> Result<Vec<u8>, i32>,
) -> Result<wire::WallpaperRequest, i32> {
    if caller != Caller::Settings {
        return Err(kerr::PERMISSION_DENIED);
    }
    let r = read(ACTION_REQUEST_MAX)
        .ok()
        .and_then(|b| wire::WallpaperRequest::decode(&b))
        .ok_or(kerr::ARGUMENT)?;
    if wallpaper::accepted(&r) {
        Ok(r)
    } else {
        Err(kerr::NOT_SUPPORTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{Activity, Trail};
    use crate::apps::{LEGACY, LEGACY_SID};
    use crate::settings::access::SETTINGS_SID;
    use alloc::vec;

    const OPS: [i32; 2] = [ipc::REPLY_TEXT, ipc::ACTIVITY];
    const CLOCK: u32 = 0xE000_8001;

    fn never(_: usize) -> Result<Vec<u8>, i32> {
        panic!("read before the caller was checked")
    }

    #[test]
    fn only_a_mapped_publisher_with_its_own_sid() {
        for op in OPS {
            assert!(is_publisher_opcode(op));
            // the legacy bridge (no catalogue SID), the kit's shared SID claimed by it, another app's SID
            assert_eq!(
                ingress(op, LEGACY, LEGACY_SID, never),
                kerr::PERMISSION_DENIED
            );
            assert_eq!(
                ingress(op, "clock", LEGACY_SID, never),
                kerr::PERMISSION_DENIED
            );
            assert_eq!(
                ingress(op, "clock", 0xE000_5001, never),
                kerr::PERMISSION_DENIED
            );
            assert_eq!(
                ingress(op, "nosuchapp", CLOCK, never),
                kerr::PERMISSION_DENIED
            );
        }
        assert!(!is_publisher_opcode(ipc::NOTIFY) && !is_publisher_opcode(ipc::TILE));
        assert!(!is_publisher_opcode(ipc::WIDGET), "WIDGET has its handler");
        assert!(!is_publisher_opcode(ipc::MEDIA), "MEDIA has its handler");
        let never_c =
            |_: usize| -> Result<Vec<u8>, i32> { panic!("read before the caller was checked") };
        assert_eq!(
            widget(LEGACY, LEGACY_SID, never_c).err(),
            Some(kerr::PERMISSION_DENIED)
        );
        assert_eq!(
            widget("clock", LEGACY_SID, never_c).err(),
            Some(kerr::PERMISSION_DENIED)
        );
        assert_eq!(
            widget("kit", CLOCK, never_c).err(),
            Some(kerr::PERMISSION_DENIED)
        );
    }

    #[test]
    fn a_mapped_publisher_gets_its_record_checked() {
        let rec = |b: Vec<u8>| {
            move |max: usize| {
                if b.len() > max {
                    Err(kerr::TOO_BIG)
                } else {
                    Ok(b)
                }
            }
        };
        let timer = Activity {
            kind: activity::kind::TIMER,
            title: "Timer".into(),
            trail: Trail::Until(5),
            ..Activity::default()
        };
        let next = widget::Content {
            name: "next".into(),
            page: 0,
            body: widget::Body::Count {
                icon: 0,
                value: 1,
                label: "x".into(),
            },
        };
        let text = widget::Content {
            body: widget::Body::Text {
                icon: 0,
                title: "07:30".into(),
                lines: vec![],
            },
            ..next.clone()
        };
        let cases = [
            (ipc::ACTIVITY, timer.encode(), kerr::NOT_SUPPORTED),
            (
                ipc::ACTIVITY,
                Activity::end(2).encode(),
                kerr::NOT_SUPPORTED,
            ),
            (
                ipc::ACTIVITY,
                Activity {
                    kind: activity::kind::MEDIA,
                    ..timer.clone()
                }
                .encode(),
                kerr::ARGUMENT,
            ),
            (ipc::ACTIVITY, vec![], kerr::ARGUMENT),
            (ipc::REPLY_TEXT, vec![], kerr::NOT_SUPPORTED),
        ];
        for (op, b, want) in cases {
            assert_eq!(
                ingress(op, "clock", CLOCK, rec(b.clone())),
                want,
                "{op} {b:?}"
            );
        }
        // REPLY_TEXT reads nothing
        assert_eq!(
            ingress(ipc::REPLY_TEXT, "clock", CLOCK, never),
            kerr::NOT_SUPPORTED
        );
        // WIDGET: the app's own content back; another template, a name not in its catalogue, another app's name
        assert_eq!(widget("clock", CLOCK, rec(text.encode())), Ok(text.clone()));
        assert_eq!(
            widget("clock", CLOCK, rec(next.encode())),
            Err(kerr::ARGUMENT)
        );
        let other = widget::Content {
            name: "other".into(),
            ..text.clone()
        };
        assert_eq!(
            widget("clock", CLOCK, rec(other.encode())),
            Err(kerr::NOT_FOUND)
        );
        assert_eq!(
            widget("files", 0xE000_6001, rec(text.encode())),
            Err(kerr::NOT_FOUND)
        );
        assert_eq!(widget("clock", CLOCK, rec(vec![1])), Err(kerr::ARGUMENT));
        assert_eq!(
            widget("clock", CLOCK, rec(vec![0; widget::WIDGET_MAX + 1])),
            Err(kerr::TOO_BIG)
        );
        // the kit's demo (its own SID)
        let demo = widget::Content {
            name: "demo".into(),
            ..text.clone()
        };
        assert_eq!(widget("kit", LEGACY_SID, rec(demo.encode())), Ok(demo));
    }

    /// (TP7) `TILE`: the caller first, then the offer.
    #[test]
    fn a_tile_offer_is_checked_and_kept() {
        let rec = |b: Vec<u8>| {
            move |max: usize| {
                if b.len() > max {
                    Err(kerr::TOO_BIG)
                } else {
                    Ok(b)
                }
            }
        };
        let o = tiles::Offer {
            local_id: 1,
            kind: tiles::kind::ACTION,
            label: "Lap".into(),
            ..tiles::Offer::default()
        };
        assert_eq!(tile("clock", CLOCK, rec(o.encode())), Ok(o));
        assert_eq!(
            tile(LEGACY, LEGACY_SID, never),
            Err(kerr::PERMISSION_DENIED)
        );
        assert_eq!(
            tile("clock", 0xE000_5001, never),
            Err(kerr::PERMISSION_DENIED)
        );
        assert_eq!(tile("clock", CLOCK, rec(vec![1, 9])), Err(kerr::ARGUMENT));
        assert_eq!(
            tile("clock", CLOCK, rec(vec![0; tiles::OFFER_MAX + 1])),
            Err(kerr::TOO_BIG)
        );
    }

    #[test]
    fn settings_actions_of_1_22() {
        let settings = crate::settings::access::caller(apps::SETTINGS, SETTINGS_SID);
        let wallpaper = wire::WallpaperRequest {
            width: 540,
            height: 1080,
        }
        .encode()
        .unwrap();
        let read = |b: &[u8]| {
            let b = b.to_vec();
            move |max: usize| {
                if b.len() > max {
                    Err(kerr::ARGUMENT)
                } else {
                    Ok(b)
                }
            }
        };
        // (D7) the wallpaper: Settings only, nothing read before; the one size the shell draws
        assert_eq!(
            wallpaper_request(Caller::App(apps::SETTINGS), never),
            Err(kerr::PERMISSION_DENIED)
        );
        assert_eq!(
            wallpaper_request(settings, read(&wallpaper)),
            Ok(wallpaper::REQUEST)
        );
        assert_eq!(
            wallpaper_request(settings, read(&[1, 1])),
            Err(kerr::ARGUMENT)
        );
        let big = wire::WallpaperRequest {
            width: 1080,
            height: 2160,
        }
        .encode()
        .unwrap();
        assert_eq!(
            wallpaper_request(settings, read(&big)),
            Err(kerr::NOT_SUPPORTED)
        );
        // (TP9) the PIN dialog: Settings only, nothing read before; a known mode
        let (good, bad) = (
            wire::encode_lock_dialog(wire::lock_dialog::CHANGE).to_vec(),
            vec![1, 3],
        );
        assert_eq!(
            lock_dialog_request(Caller::App("clock"), never),
            Err(kerr::PERMISSION_DENIED)
        );
        // an app that calls itself settings without NEOSET's SID is an app
        assert_eq!(
            lock_dialog_request(Caller::App(apps::SETTINGS), never),
            Err(kerr::PERMISSION_DENIED)
        );
        assert_eq!(
            lock_dialog_request(settings, read(&good)),
            Ok(wire::lock_dialog::CHANGE)
        );
        assert_eq!(
            lock_dialog_request(settings, read(&bad)),
            Err(kerr::ARGUMENT)
        );
        assert_eq!(
            lock_dialog_request(settings, read(&[0; ACTION_REQUEST_MAX + 1])),
            Err(kerr::ARGUMENT)
        );
    }

    #[test]
    fn media_records() {
        let song = media::Session {
            state: media::state::PLAYING,
            title: "x".into(),
            ..media::Session::default()
        };
        let b = song.encode();
        assert_eq!(media("clock", CLOCK, |_| Ok(b.clone())), Ok(Some(song)));
        assert_eq!(media("clock", CLOCK, |_| Ok(vec![])), Ok(None));
        assert_eq!(media("clock", CLOCK, |_| Ok(vec![1])), Err(kerr::ARGUMENT));
        assert_eq!(
            media("clock", CLOCK, |_| Err(kerr::TOO_BIG)),
            Err(kerr::TOO_BIG)
        );
        assert_eq!(
            media(LEGACY, LEGACY_SID, never),
            Err(kerr::PERMISSION_DENIED)
        );
        assert_eq!(
            media("clock", 0xE000_5001, never),
            Err(kerr::PERMISSION_DENIED)
        );
    }
}
