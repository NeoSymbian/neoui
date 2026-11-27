// SPDX-License-Identifier: Apache-2.0
//! How SYSSTART checks protocol 1.22's opcodes and Settings actions (W0a; docs neoui-shell/protocol.md "Protocol
//! 1.22"): the caller first, then its record. Each stream replaced the contract's `KErrNotSupported` with its handler
//! and kept the checks: `WIDGET` is [`widget`] (home Part 4), `MEDIA` [`media`] (TP3), `TILE` [`tile`] (TP7),
//! `ACTIVITY` [`activity`] (TP6) and `REPLY_TEXT` [`publisher`] (TP4).

use crate::settings::access::Caller;
use crate::settings::kerr;
use crate::settings_app::{wallpaper, wire};
use crate::{activity, apps, ipc, media, tiles, widget};
use alloc::vec::Vec;

/// The caller of a publishing opcode: `KErrPermissionDenied` unless app `key` has a catalogue SID and the message's
/// secure ID `sid` is it (the legacy bridge and the kit's shared SID publish nothing). `REPLY_TEXT` (TP4) is checked
/// so before anything is read.
///
/// # Errors
/// As above.
pub fn publisher(key: &str, sid: u32) -> Result<(), i32> {
    if apps::sid(key) == Some(sid) {
        Ok(())
    } else {
        Err(kerr::PERMISSION_DENIED)
    }
}

/// (TP7) A `TILE` from app `key` whose message carries secure ID `sid`, checked as the others: [`publisher`], then
/// `KErrArgument` for a malformed offer (`read` refuses one past [`tiles::OFFER_MAX`] with `KErrTooBig`).
///
/// # Errors
/// As above.
pub fn tile(
    key: &str,
    sid: u32,
    read: impl FnOnce(usize) -> Result<Vec<u8>, i32>,
) -> Result<tiles::Offer, i32> {
    publisher(key, sid)?;
    tiles::Offer::decode(&read(tiles::OFFER_MAX)?).ok_or(kerr::ARGUMENT)
}

/// (TP6) An `ACTIVITY` from app `key` whose message carries secure ID `sid`: [`publisher`] (`read` is called only
/// then, refusing a record over its maximum), then `KErrArgument` for a malformed record, and for kind Media too
/// (that one is the shell's, derived from the media session).
///
/// # Errors
/// As above.
pub fn activity(
    key: &str,
    sid: u32,
    read: impl FnOnce(usize) -> Result<Vec<u8>, i32>,
) -> Result<activity::Activity, i32> {
    publisher(key, sid)?;
    activity::Activity::decode(&read(activity::ACTIVITY_MAX)?)
        .filter(|a| a.kind != activity::kind::MEDIA)
        .ok_or(kerr::ARGUMENT)
}

/// `WIDGET` (46) from app `key` whose message carries secure ID `sid`: the content to keep, checked as for
/// [`publisher`] (the caller first, `read` only then): `KErrPermissionDenied` unless `key` has a catalogue SID and `sid`
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
    publisher(key, sid)?;
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
    publisher(key, sid)?;
    let b = read(media::MEDIA_MAX)?;
    if b.is_empty() {
        return Ok(None);
    }
    media::Session::decode(&b).map(Some).ok_or(kerr::ARGUMENT)
}

/// The longest request [`settings_action`] reads.
pub const ACTION_REQUEST_MAX: usize = wire::WallpaperRequest::LEN;

/// The answer to `SETTINGS_ACTION` `kind` when it is one of 1.22's not built yet (14 `LOCK_DIALOG`), else `None`.
/// `read(max)` reads the request (slot 1, `KErrArgument` past `max`), only for Settings. `KErrPermissionDenied` for
/// anyone else, `KErrArgument` for a malformed request, else `KErrNotSupported`. 8 `INSTALL_WALLPAPER` is built (D7):
/// [`wallpaper_request`]; 12 `EDIT_TILES` is built with TP7, the shell's `edit_tiles_action`.
pub fn settings_action(
    kind: i32,
    caller: Caller<'_>,
    read: impl FnOnce(usize) -> Result<Vec<u8>, i32>,
) -> Option<i32> {
    let valid: fn(&[u8]) -> bool = match kind {
        ipc::ACTION_LOCK_DIALOG => |b| wire::lock_dialog_request(b).is_some(),
        _ => return None,
    };
    if caller != Caller::Settings {
        return Some(kerr::PERMISSION_DENIED);
    }
    Some(match read(ACTION_REQUEST_MAX) {
        Ok(b) if valid(&b) => kerr::NOT_SUPPORTED,
        _ => kerr::ARGUMENT,
    })
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

    const CLOCK: u32 = 0xE000_8001;

    fn never(_: usize) -> Result<Vec<u8>, i32> {
        panic!("read before the caller was checked")
    }

    #[test]
    fn only_a_mapped_publisher_with_its_own_sid() {
        // the legacy bridge (no catalogue SID), the kit's shared SID claimed by it, another app's SID
        for (key, sid) in [
            (LEGACY, LEGACY_SID),
            ("clock", LEGACY_SID),
            ("clock", 0xE000_5001),
            ("nosuchapp", CLOCK),
        ] {
            assert_eq!(publisher(key, sid), Err(kerr::PERMISSION_DENIED));
            assert_eq!(activity(key, sid, never), Err(kerr::PERMISSION_DENIED));
        }
        assert_eq!(publisher("clock", CLOCK), Ok(()));
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
        assert_eq!(
            activity("clock", CLOCK, rec(timer.encode())),
            Ok(timer.clone())
        );
        assert_eq!(
            activity("clock", CLOCK, rec(Activity::end(2).encode())),
            Ok(Activity::end(2))
        );
        let media = Activity {
            kind: activity::kind::MEDIA,
            ..timer.clone()
        };
        assert_eq!(
            activity("clock", CLOCK, rec(media.encode())),
            Err(kerr::ARGUMENT),
            "the media activity is the shell's"
        );
        assert_eq!(activity("clock", CLOCK, rec(vec![])), Err(kerr::ARGUMENT));
        assert_eq!(
            activity("clock", CLOCK, rec(vec![1; activity::ACTIVITY_MAX + 1])),
            Err(kerr::TOO_BIG)
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
        assert_eq!(
            settings_action(ipc::ACTION_INSTALL_WALLPAPER, settings, never),
            None
        );
        // the one action 1.22 still checks without acting on it
        {
            let (kind, good, bad) = (
                ipc::ACTION_LOCK_DIALOG,
                wire::encode_lock_dialog(wire::lock_dialog::CHANGE).to_vec(),
                vec![1, 3],
            );
            assert_eq!(
                settings_action(kind, Caller::App("clock"), never),
                Some(kerr::PERMISSION_DENIED)
            );
            // an app that calls itself settings without NEOSET's SID is an app
            assert_eq!(
                settings_action(kind, Caller::App(apps::SETTINGS), never),
                Some(kerr::PERMISSION_DENIED)
            );
            assert_eq!(
                settings_action(kind, settings, read(&good)),
                Some(kerr::NOT_SUPPORTED)
            );
            assert_eq!(
                settings_action(kind, settings, read(&bad)),
                Some(kerr::ARGUMENT)
            );
            assert_eq!(
                settings_action(kind, settings, read(&[0; ACTION_REQUEST_MAX + 1])),
                Some(kerr::ARGUMENT)
            );
        }
        for kind in [
            ipc::ACTION_EDIT_TILES,
            ipc::ACTION_SIGN_IN,
            ipc::ACTION_EDIT_HOME,
            ipc::ACTION_VOLUME,
        ] {
            assert_eq!(settings_action(kind, settings, never), None);
        }
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
