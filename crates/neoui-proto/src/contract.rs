// SPDX-License-Identifier: Apache-2.0
//! What SYSSTART answers for protocol 1.22's opcodes and Settings actions before their shell side is built (W0a; docs
//! neoui-shell/protocol.md "Protocol 1.22"): the caller is checked first, then its record, and a request that would
//! be accepted answers `KErrNotSupported`. Each later stream replaces its part with the real handler and keeps the
//! checks: `WIDGET` is [`widget`] (home Part 4, W1-E).

use crate::settings::access::Caller;
use crate::settings::kerr;
use crate::{activity, apps, ipc, media, settings_app::wire, tiles, widget};
use alloc::vec::Vec;

/// Whether `op` is one of 1.22's app → shell opcodes that publish for the caller (checked by [`ingress`]).
#[must_use]
pub fn is_publisher_opcode(op: i32) -> bool {
    matches!(op, ipc::MEDIA | ipc::REPLY_TEXT | ipc::ACTIVITY | ipc::TILE)
}

/// The answer to publisher opcode `op` from app `key` whose message carries secure ID `sid`. `read(max)` reads the
/// record (slot 0), refusing one over `max` bytes; it is called only once the caller is known.
///
/// `KErrPermissionDenied` unless `key` has a catalogue SID and `sid` is it (the legacy bridge and the kit's shared
/// SID publish nothing); then `KErrArgument` for a malformed record (an ACTIVITY of kind Media too: that one is the
/// shell's), `KErrNotFound` for a widget name not in the app's catalogue; else `KErrNotSupported`.
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
        ipc::MEDIA => read(media::MEDIA_MAX)
            .and_then(|b| ok(b.is_empty() || media::Session::decode(&b).is_some())),
        ipc::ACTIVITY => read(activity::ACTIVITY_MAX).and_then(|b| {
            ok(activity::Activity::decode(&b).is_some_and(|a| a.kind != activity::kind::MEDIA))
        }),
        ipc::TILE => read(tiles::OFFER_MAX).and_then(|b| ok(tiles::Offer::decode(&b).is_some())),
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

/// The longest request [`settings_action`] reads.
pub const ACTION_REQUEST_MAX: usize = wire::WallpaperRequest::LEN;

/// The answer to `SETTINGS_ACTION` `kind` when it is one of 1.22's (8 `INSTALL_WALLPAPER`, 12 `EDIT_TILES`, 14
/// `LOCK_DIALOG`), else `None`. `read(max)` reads the request (slot 1, `KErrArgument` past `max`), only for
/// Settings. `KErrPermissionDenied` for anyone else, `KErrArgument` for a malformed request, else
/// `KErrNotSupported`.
pub fn settings_action(
    kind: i32,
    caller: Caller<'_>,
    read: impl FnOnce(usize) -> Result<Vec<u8>, i32>,
) -> Option<i32> {
    let valid: fn(&[u8]) -> bool = match kind {
        ipc::ACTION_INSTALL_WALLPAPER => |b| wire::WallpaperRequest::decode(b).is_some(),
        ipc::ACTION_EDIT_TILES => tiles::edit_tiles_request,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{Activity, Trail};
    use crate::apps::{LEGACY, LEGACY_SID};
    use crate::settings::access::SETTINGS_SID;
    use alloc::vec;

    const OPS: [i32; 4] = [ipc::MEDIA, ipc::REPLY_TEXT, ipc::ACTIVITY, ipc::TILE];
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
        assert!(!is_publisher_opcode(ipc::NOTIFY));
        assert!(!is_publisher_opcode(ipc::WIDGET), "WIDGET has its handler");
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
        let song = media::Session {
            state: media::state::PLAYING,
            title: "x".into(),
            ..media::Session::default()
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
            (ipc::MEDIA, song.encode(), kerr::NOT_SUPPORTED),
            (ipc::MEDIA, vec![], kerr::NOT_SUPPORTED),
            (ipc::MEDIA, vec![1], kerr::ARGUMENT),
            (ipc::MEDIA, vec![0; media::MEDIA_MAX + 1], kerr::TOO_BIG),
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
            (
                ipc::TILE,
                tiles::Offer::withdraw(1).encode(),
                kerr::NOT_SUPPORTED,
            ),
            (ipc::TILE, vec![1, 9], kerr::ARGUMENT),
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
        for (kind, good, bad) in [
            (ipc::ACTION_INSTALL_WALLPAPER, wallpaper.clone(), vec![1, 1]),
            (ipc::ACTION_EDIT_TILES, vec![], vec![0]),
            (
                ipc::ACTION_LOCK_DIALOG,
                wire::encode_lock_dialog(wire::lock_dialog::CHANGE).to_vec(),
                vec![1, 3],
            ),
        ] {
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
            ipc::ACTION_SIGN_IN,
            ipc::ACTION_EDIT_HOME,
            ipc::ACTION_VOLUME,
        ] {
            assert_eq!(settings_action(kind, settings, never), None);
        }
    }
}
