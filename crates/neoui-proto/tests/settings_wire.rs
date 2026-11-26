// SPDX-License-Identifier: Apache-2.0
//! Protocol 1.20's Settings contracts (docs neoui-shell/protocol.md "Settings snapshots, owner reports and panel
//! policy"): golden bytes for every new payload and event, the paging rules, the old kinds and event 28 unchanged, and
//! the owner-work state machine.

use neoui_proto::apps;
use neoui_proto::battery::Usage;
use neoui_proto::ipc::{self, AppEvent, EVENT_LEN};
use neoui_proto::settings::kerr;
use neoui_proto::settings_app::info::{self, Battery, Clock, OldApps};
use neoui_proto::settings_app::jobs::{self, Jobs};
use neoui_proto::settings_app::wire::{
    self, AppDetail, AppRef, BatteryCycle, ClearRequest, Dedup, DriveUsage, Features, Job,
    MeasureRequest, OwnerReport, OwnerWork, Revisions, Traversal, scope, state,
};
use neoui_proto::wifi::{Internet, Ip};

const T: [u8; 8] = [0x00, 0xF1, 0x53, 0x65, 0, 0, 0, 0]; // 1_700_000_000
const UTC: i64 = 1_700_000_000;

fn ev(w: [u32; 4]) -> [u8; EVENT_LEN] {
    let mut b = [0; EVENT_LEN];
    for (i, v) in w.iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    b
}

/// `decode` takes `whole` and refuses every shorter prefix and one trailing byte.
fn strict<T: PartialEq + core::fmt::Debug>(whole: &[u8], decode: impl Fn(&[u8]) -> Option<T>) -> T {
    for n in 0..whole.len() {
        assert_eq!(decode(&whole[..n]), None, "prefix of {n} bytes");
    }
    assert_eq!(decode(&[whole, &[0]].concat()), None, "a trailing byte");
    decode(whole).expect("the whole record")
}

// --- events -----------------------------------------------------------------------------------------------------

#[test]
fn events_1_20_golden_and_old_28_unchanged() {
    let cases = [
        (
            AppEvent::SettingsInfo {
                kind: 13,
                rev: 0x0102_0304,
            },
            [27, 13, 0x0102_0304, 0],
        ),
        (AppEvent::OwnerWork { ticket: 9 }, [29, 9, 0, 0]),
        (
            AppEvent::ClearTicket {
                ticket: 0x7fff_ffff,
            },
            [28, 0x7fff_ffff, 0, 0],
        ),
        (AppEvent::ClearData, [28, 0, 0, 0]),
    ];
    for (e, w) in cases {
        assert_eq!(e.encode(), ev(w), "{e:?}");
        assert_eq!(AppEvent::decode(&ev(w)), Some(e));
    }
    // the 1.17 event's bytes, as Clock and Calendar 1.19 decode them
    assert_eq!(
        AppEvent::ClearData.encode(),
        [28, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        AppEvent::decode(&ev([27, 256, 1, 0])),
        None,
        "a kind over u8"
    );
    // never to an older client
    assert_eq!(AppEvent::ClearData.needs_minor(), 0);
    assert_eq!(AppEvent::NotifyGone { id: 1, why: 0 }.needs_minor(), 18);
    for e in [
        AppEvent::SettingsInfo { kind: 13, rev: 1 },
        AppEvent::OwnerWork { ticket: 1 },
        AppEvent::ClearTicket { ticket: 1 },
    ] {
        assert_eq!(e.needs_minor(), 20);
    }
    assert_eq!(ipc::VERSION, (1, 20));
}

// --- kinds 1–6: no wrapper ----------------------------------------------------------------------------------------

#[test]
fn info_1_to_6_keep_their_bytes() {
    // 1 battery: [save_error][1440 slots]
    let mut bars = vec![-1.0; info::BATTERY_SLOTS];
    bars[0] = 0.5;
    let b = Battery {
        save_error: false,
        bars,
    }
    .encode();
    assert_eq!(b.len(), 1441);
    assert_eq!(&b[..4], &[0, 50, 255, 255]);
    // 2 the IP side
    let ip = Ip {
        address: Some(([192, 168, 3, 57], 24)),
        gateway: [192, 168, 3, 1],
        dns: [8, 8, 8, 8],
        internet: Internet::Ok,
    };
    assert_eq!(
        info::encode_ip(&ip),
        [1, 192, 168, 3, 57, 24, 192, 168, 3, 1, 8, 8, 8, 8, 1]
    );
    // 3 the clock
    let c = Clock {
        zone: "UTC".into(),
        synced: "Not yet".into(),
    };
    assert_eq!(c.encode(), b"\x03UTC\x07Not yet");
    // 4 the old apps: [count][total u16][id u32][len][caption]
    let o = OldApps {
        total: 3,
        apps: vec![(0x1000_0001, "X".into())],
    };
    assert_eq!(o.encode(), [1, 3, 0, 0x01, 0, 0, 0x10, 1, b'X']);
    // 5 battery usage: [0] or the 28-byte record
    assert_eq!(info::encode_usage(None), [0]);
    let u = Usage {
        since_full: true,
        start_utc: UTC,
        end_utc: UTC + 1,
        start_percent: Some(100),
        end_percent: None,
        on_battery_s: 2,
        gap_s: 3,
    };
    let mut want = vec![1, 1];
    want.extend_from_slice(&T);
    want.extend_from_slice(&[0x01, 0xF1, 0x53, 0x65, 0, 0, 0, 0]);
    want.extend_from_slice(&[100, 255, 2, 0, 0, 0, 3, 0, 0, 0]);
    assert_eq!(info::encode_usage(Some(&u)), want);
    // 6 the running apps: [count] then [resident][len][key]
    assert_eq!(
        info::encode_running(&[("phone", true)]),
        [1, 1, 5, b'p', b'h', b'o', b'n', b'e']
    );
}

// --- paging --------------------------------------------------------------------------------------------------------

fn recs(n: usize, len: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| vec![i as u8; len]).collect()
}

#[test]
fn an_empty_collection_is_one_terminal_header() {
    let p = wire::page(13, 1, &[], 0, 0, wire::PAGE_MAX).unwrap();
    assert_eq!(
        p,
        [1, 13, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]
    );
    let (h, r) = wire::parse_page(&p, 13, 0, 0).unwrap();
    assert_eq!((h.count, h.next, r.len()), (0, wire::TERMINAL, 0));
}

#[test]
fn first_middle_last_pages() {
    let records = recs(5, 3); // 5 bytes each on the wire
    let max = wire::HEADER_LEN + 2 * 5; // two per page
    let p0 = wire::page(8, 7, &records, 0, 0, max).unwrap();
    assert_eq!(
        p0,
        [
            1, 8, 2, 0, 7, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0,
            0, // header: count 2, rev 7, first 0, next 2
            3, 0, 0, 0, 0, 3, 0, 1, 1, 1
        ]
    );
    let mut t = Traversal::new(8);
    assert_eq!(t.request(), (0, 0));
    assert_eq!(t.feed(&p0), Some(false));
    assert_eq!(t.request(), (2, 7));
    let p1 = wire::page(8, 7, &records, 2, 7, max).unwrap();
    assert_eq!(t.feed(&p1), Some(false));
    let p2 = wire::page(8, 7, &records, 4, 7, max).unwrap();
    let (h, _) = wire::parse_page(&p2, 8, 4, 7).unwrap();
    assert_eq!((h.count, h.next), (1, wire::TERMINAL));
    assert_eq!(t.feed(&p2), Some(true));
    assert_eq!(t.finish(), (7, records));
}

#[test]
fn exactly_full_and_one_byte_short() {
    let records = recs(2, 10); // 12 bytes each
    let full = wire::HEADER_LEN + 24;
    let p = wire::page(13, 1, &records, 0, 0, full).unwrap();
    assert_eq!(p.len(), full);
    assert_eq!(
        wire::parse_page(&p, 13, 0, 0).unwrap().0.next,
        wire::TERMINAL
    );
    // one byte short: the second record waits for the next page
    let p = wire::page(13, 1, &records, 0, 0, full - 1).unwrap();
    let (h, _) = wire::parse_page(&p, 13, 0, 0).unwrap();
    assert_eq!((h.count, h.next), (1, 1));
    // not even one record, or not the header: KErrOverflow, nothing written
    assert_eq!(
        wire::page(13, 1, &records, 0, 0, wire::HEADER_LEN + 11),
        Err(kerr::OVERFLOW)
    );
    assert_eq!(wire::page(13, 1, &[], 0, 0, 15), Err(kerr::OVERFLOW));
    // a page is never more than 4096 bytes, whatever the descriptor
    let many = recs(600, 10);
    assert!(wire::page(13, 1, &many, 0, 0, 1 << 20).unwrap().len() <= wire::PAGE_MAX);
}

#[test]
fn stale_revisions_and_bad_cursors() {
    let records = recs(3, 1);
    assert_eq!(
        wire::page(13, 5, &records, 1, 4, 100),
        Err(wire::KERR_NOT_READY)
    );
    assert_eq!(
        wire::page(13, 5, &records, 1, 0, 100),
        Err(kerr::ARGUMENT),
        "a cursor without a revision"
    );
    assert_eq!(
        wire::page(13, 5, &records, 3, 5, 100),
        Err(kerr::ARGUMENT),
        "past the end"
    );
    assert_eq!(wire::page(13, 5, &records, 9, 5, 100), Err(kerr::ARGUMENT));
    // the client: a changed revision, a cursor that doesn't move, a next that skips, a zero revision, another
    // kind, another schema, an unechoed first
    let good = wire::page(13, 5, &records, 0, 0, wire::HEADER_LEN + 3).unwrap();
    assert!(wire::parse_page(&good, 13, 0, 0).is_some());
    assert!(wire::parse_page(&good, 13, 0, 6).is_none());
    let patch = |at: usize, v: &[u8]| {
        let mut b = good.clone();
        b[at..at + v.len()].copy_from_slice(v);
        b
    };
    assert!(
        wire::parse_page(&patch(2, &[0, 0]), 13, 0, 0).is_none(),
        "count 0, not terminal"
    );
    assert!(
        wire::parse_page(&patch(12, &[0, 0, 0, 0]), 13, 0, 0).is_none(),
        "next = first"
    );
    assert!(
        wire::parse_page(&patch(12, &[2, 0, 0, 0]), 13, 0, 0).is_none(),
        "next skips"
    );
    assert!(
        wire::parse_page(&patch(4, &[0, 0, 0, 0]), 13, 0, 0).is_none(),
        "rev 0"
    );
    assert!(wire::parse_page(&good, 8, 0, 0).is_none());
    assert!(wire::parse_page(&patch(0, &[2]), 13, 0, 0).is_none());
    assert!(wire::parse_page(&good, 13, 1, 0).is_none());
    for n in 0..good.len() {
        assert!(
            wire::parse_page(&good[..n], 13, 0, 0).is_none(),
            "cut at {n}"
        );
    }
    assert!(wire::parse_page(&[&good[..], &[0]].concat(), 13, 0, 0).is_none());
    // a traversal fed a page from another revision drops out
    let mut t = Traversal::new(13);
    assert_eq!(t.feed(&good), Some(false));
    let other = wire::page(13, 6, &records, 1, 6, 100).unwrap();
    assert_eq!(t.feed(&other), None);
}

#[test]
fn revisions_start_at_one_and_skip_zero_on_wrap() {
    let mut r = Revisions::default();
    assert_eq!(r.get(13), 1);
    assert_eq!(r.bump(13), 2);
    assert_eq!(r.get(8), 1, "per kind");
    let mut r = Revisions([u32::MAX; 16]);
    assert_eq!(r.bump(13), 1);
    assert_eq!(r.get(99), 1);
}

#[test]
fn a_traversal_is_capped() {
    let mut t = Traversal::new(13);
    let mut first = 0u32;
    let mut rev = 0;
    let records = recs(wire::TRAVERSAL_RECORDS_MAX + 1, 0);
    loop {
        let p = wire::page(13, 1, &records, first, rev, wire::PAGE_MAX).unwrap();
        match t.feed(&p) {
            Some(false) => (first, rev) = t.request(),
            Some(true) => panic!("more than the cap went through"),
            None => break,
        }
    }
}

// --- records -------------------------------------------------------------------------------------------------------

fn detail_bytes() -> Vec<u8> {
    [
        &[0u8, 5][..],
        b"files",
        &[5],
        b"Files",
        &[8],
        b"neofiles",
        &[3],
        b"1.0",
        &[1],                      // identity: catalogue
        &[0x01, 0x60, 0x00, 0xE0], // sid
        &[1],                      // caps from the E32 header
        &[2, 0, 0, 0, 1, 0, 0, 0], // caps lo, hi
        &[3],                      // coverage complete
        &T,
        &[0x02, 0x01, 0, 0], // clear scope: cache | files index
        &[1],                // one drive
        &[2, 0x0B],          // C:, installed, data, logs valid
        &[1, 0, 0, 0, 0, 0, 0, 0],
        &[2, 0, 0, 0, 0, 0, 0, 0],
        &[0; 8],
        &[4, 0, 0, 0, 0, 0, 0, 0],
    ]
    .concat()
}

fn detail() -> AppDetail {
    AppDetail {
        app: AppRef::Native("files"),
        caption: "Files".into(),
        exe: "neofiles".into(),
        version: "1.0".into(),
        identity_source: 1,
        sid: 0xE000_6001,
        cap_source: 1,
        caps: 0x1_0000_0002,
        coverage: wire::coverage::COMPLETE,
        measured_utc: UTC,
        clear_scope: scope::CACHE | scope::FILES_INDEX,
        drives: vec![DriveUsage {
            drive: 2,
            installed: Some(1),
            data: Some(2),
            cache: None,
            logs: Some(4),
        }],
    }
}

#[test]
fn app_detail_golden_and_strict() {
    assert_eq!(detail().encode(), detail_bytes());
    assert_eq!(strict(&detail_bytes(), AppDetail::decode), detail());
    let legacy = AppDetail {
        app: AppRef::Legacy(0x1000_0001),
        drives: vec![],
        ..detail()
    };
    assert_eq!(&legacy.encode()[..5], &[1, 0x01, 0, 0, 0x10]);
    assert_eq!(AppDetail::decode(&legacy.encode()), Some(legacy));
    let bad = |at: usize, v: u8| {
        let mut b = detail_bytes();
        b[at] = v;
        AppDetail::decode(&b)
    };
    assert_eq!(bad(0, 2), None, "app tag");
    assert_eq!(bad(2, b'x'), None, "not a catalogue key");
    assert_eq!(bad(8, 0xff), None, "bad UTF-8 caption");
    let at = detail_bytes().len() - 34;
    assert_eq!(bad(at, 26), None, "drive past Z:");
    assert_eq!(bad(at + 1, 0x10), None, "validity bit 4");
    assert_eq!(
        bad(at + 2 + 16, 1),
        None,
        "a count without its validity bit"
    );
    let scope_at = at - 5;
    assert_eq!(bad(scope_at + 1, 0x02), None, "reserved scope bit");
    // the same drive twice, the same app twice
    let two = AppDetail {
        drives: vec![detail().drives[0]; 2],
        ..detail()
    };
    assert_eq!(AppDetail::decode(&two.encode()), None);
    let r = vec![detail().encode(), detail().encode()];
    assert_eq!(wire::decode_details(&r), None);
    assert_eq!(wire::decode_details(&r[..1]), Some(vec![detail()]));
}

#[test]
fn battery_cycle_golden_and_strict() {
    let c = BatteryCycle {
        flags: wire::cycle::FULL_THEN_DISCHARGE | wire::cycle::SCREEN_VALID,
        start_percent: Some(100),
        end_percent: None,
        start_utc: UTC,
        last_utc: UTC,
        on_battery_s: 1,
        screen_on_battery_s: 2,
        gap_s: 3,
    };
    let want = [
        &[0x09, 100, 255, 0][..],
        &T,
        &T,
        &[1, 0, 0, 0, 0, 0, 0, 0],
        &[2, 0, 0, 0, 0, 0, 0, 0],
        &[3, 0, 0, 0, 0, 0, 0, 0],
    ]
    .concat();
    assert_eq!(c.encode(), want);
    assert_eq!(strict(&want, BatteryCycle::decode), c);
    for (at, v) in [(0, 0x10), (1, 101), (3, 1)] {
        let mut b = want.clone();
        b[at] = v;
        assert_eq!(BatteryCycle::decode(&b), None, "byte {at}");
    }
    assert_eq!(
        wire::decode_singleton(&[], BatteryCycle::decode),
        Some(None)
    );
    assert_eq!(
        wire::decode_singleton(std::slice::from_ref(&want), BatteryCycle::decode),
        Some(Some(c))
    );
    assert_eq!(
        wire::decode_singleton(&[want.clone(), want], BatteryCycle::decode),
        None
    );
}

fn job() -> Job {
    Job {
        ticket: 5,
        action: ipc::ACTION_CLEAR as u8,
        state: state::PARTIAL,
        subject: Some(AppRef::Native("clock")),
        requested_scope: scope::CLOCK | scope::CALENDAR,
        committed_scope: scope::CLOCK,
        result: -1,
        finished_utc: UTC,
    }
}

#[test]
fn job_golden_and_strict() {
    let want = [
        &[5u8, 0, 0, 0, 6, 3, 0, 0, 1, 0, 5][..],
        b"clock",
        &[0xC0, 0, 0, 0, 0x40, 0, 0, 0, 0xff, 0xff, 0xff, 0xff],
        &T,
    ]
    .concat();
    assert_eq!(job().encode(), want);
    assert_eq!(strict(&want, Job::decode), job());
    let none = Job {
        subject: None,
        ..job()
    };
    assert_eq!(Job::decode(&none.encode()), Some(none));
    let bad = |j: Job| Job::decode(&j.encode());
    assert_eq!(bad(Job { ticket: 0, ..job() }), None);
    assert_eq!(
        bad(Job {
            ticket: 0x8000_0000,
            ..job()
        }),
        None
    );
    assert_eq!(bad(Job { state: 6, ..job() }), None);
    assert_eq!(
        bad(Job {
            committed_scope: scope::SMS,
            ..job()
        }),
        None,
        "more than asked"
    );
    let mut b = want.clone();
    b[6] = 1;
    assert_eq!(Job::decode(&b), None, "reserved bytes");
    assert_eq!(
        wire::decode_jobs(&[want.clone(), want]),
        None,
        "the same ticket twice"
    );
}

#[test]
fn features_golden_and_what_is_advertised() {
    let want = [
        &[1u8, 20, 0, 0][..],
        &[0x7E, 0x63, 0, 0, 0, 0, 0, 0], // INFO 1–6, 8, 9, 13, 14
        &[0xFE, 0, 0, 0, 0, 0, 0, 0],    // actions 1–7
        &[0xFE, 0xE7, 0xFF, 0x7F, 0x01, 0x3F, 0, 0], // opcodes 1–10, 13–30, 32, 40–45
    ]
    .concat();
    assert_eq!(Features::SHELL.encode(), want);
    assert_eq!(strict(&want, Features::decode), Features::SHELL);
    let f = Features::SHELL;
    // a kind without its producer isn't advertised, nor is anything reserved
    for kind in [
        ipc::INFO_CHANNELS,
        ipc::INFO_VOLUME_STATE,
        ipc::INFO_UPDATE_STATUS,
        ipc::INFO_TILES,
        ipc::INFO_ENERGY_SHARES,
    ] {
        assert!(!f.has_info(kind), "INFO {kind}");
    }
    assert!(f.has_info(ipc::INFO_JOBS) && f.has_info(ipc::INFO_FEATURES));
    assert!(
        f.has_info(ipc::INFO_BATTERY_CYCLE),
        "D5's producer is built"
    );
    assert!(f.has_info(ipc::INFO_APP_DETAIL), "D4 produces INFO 8");
    for a in ipc::ACTION_INSTALL_WALLPAPER..=ipc::ACTION_LOCK_DIALOG {
        assert!(!f.has_action(a), "action {a}");
    }
    for op in [
        11,
        12,
        31,
        ipc::CHANNEL,
        ipc::REPLY_TEXT,
        36,
        ipc::TILE,
        38,
        39,
    ] {
        assert!(!f.has_opcode(op), "opcode {op}");
    }
    assert!(
        f.has_opcode(ipc::OWNER_REPORT)
            && f.has_opcode(ipc::ENTRY_THUMB)
            && f.has_opcode(ipc::CONTROL_FEEDBACK)
    );
}

#[test]
fn requests_golden_and_strict() {
    let nonce = [8u8, 7, 6, 5, 4, 3, 2, 1];
    let c = ClearRequest {
        nonce: 0x0102_0304_0506_0708,
        app: AppRef::Native("clock"),
        scope: scope::CLOCK,
    };
    let want = [&[0xff, 1][..], &nonce, &[0, 5], b"clock", &[0x40, 0, 0, 0]].concat();
    assert_eq!(c.encode(), want);
    assert_eq!(strict(&want, ClearRequest::decode), c);
    assert_eq!(
        ClearRequest::decode(&ClearRequest { nonce: 0, ..c }.encode()),
        None
    );
    assert_eq!(
        ClearRequest::decode(&ClearRequest { scope: 0, ..c }.encode()),
        None
    );
    // the 1.17 form is a bare key: never the confirmed one
    assert_ne!(b"clock"[0], wire::CONFIRMED);
    assert_eq!(ClearRequest::decode(b"clock"), None);

    let m = MeasureRequest {
        nonce: 0x0102_0304_0506_0708,
        app: AppRef::Legacy(0x2000_0002),
    };
    let want = [&[1u8][..], &nonce, &[1, 0x02, 0, 0, 0x20]].concat();
    assert_eq!(m.encode(), want);
    assert_eq!(strict(&want, MeasureRequest::decode), m);
    assert!(want.len() <= wire::REQUEST_MAX);
}

#[test]
fn owner_payloads_golden_and_strict() {
    assert_eq!(wire::encode_register(3), [1, 3]);
    assert_eq!(wire::decode_register(&[1, 3]), Some(3));
    for bad in [&[1u8, 0][..], &[1, 4], &[2, 1], &[1], &[1, 1, 0]] {
        assert_eq!(wire::decode_register(bad), None, "{bad:?}");
    }
    let w = OwnerWork {
        work: wire::WORK_CLEAR,
        subject: AppRef::Native("clock"),
        scope: scope::CLOCK,
    };
    let want = [&[1u8, 1, 0, 5][..], b"clock", &[0x40, 0, 0, 0]].concat();
    assert_eq!(w.encode(), want);
    assert_eq!(strict(&want, OwnerWork::decode), w);

    let r = OwnerReport {
        state: state::COMMITTED,
        committed_scope: 0,
        result: 0,
        coverage: wire::coverage::PARTIAL,
        measured_utc: UTC,
        version: "2".into(),
        drives: vec![DriveUsage {
            drive: 4,
            data: Some(9),
            ..DriveUsage::default()
        }],
    };
    let want = [
        &[1u8, 2, 0, 0, 0, 0, 0, 0, 0, 0, 2][..],
        &T,
        &[1, b'2', 1, 4, 0x02],
        &[0; 8],
        &[9, 0, 0, 0, 0, 0, 0, 0],
        &[0; 16],
    ]
    .concat();
    assert_eq!(r.encode(), want);
    assert_eq!(strict(&want, OwnerReport::decode), r);
    for st in [state::QUEUED, state::RUNNING, 6] {
        let mut b = want.clone();
        b[1] = st;
        assert_eq!(OwnerReport::decode(&b), None, "state {st}");
    }
}

#[test]
fn thumbnails() {
    // small icons pass as they are; the generic and malformed ones are empty
    let small = [2u8, 0, 1, 0, 0x33, 0x22, 0x11, 0x80, 0xff, 0xff, 0xff, 0];
    assert_eq!(wire::thumb(&small), small);
    assert!(wire::thumb(&[]).is_empty());
    assert!(wire::thumb(&[1, 0]).is_empty());
    // 128 × 64 → 32 × 16, every fourth pixel
    let mut icon = vec![128, 0, 64, 0];
    for y in 0..64u32 {
        for x in 0..128u32 {
            icon.extend_from_slice(&(y << 8 | x).to_le_bytes());
        }
    }
    let t = wire::thumb(&icon);
    let (w, h, px) = wire::decode_thumb(&t).unwrap();
    assert_eq!((w, h), (32, 16));
    assert_eq!(&px[4..8], &(4u32).to_le_bytes());
    assert_eq!(&px[32 * 4..32 * 4 + 4], &(4u32 << 8).to_le_bytes());
    assert!(t.len() <= wire::THUMB_MAX);
    assert_eq!(wire::decode_thumb(&icon), None, "over 32");
}

#[test]
fn nonces_dedup() {
    let mut d = Dedup::default();
    assert_eq!(d.check(1, 7, b"a"), Ok(None));
    d.record(1, 7, b"a", 3);
    assert_eq!(d.check(1, 7, b"a"), Ok(Some(3)), "a retry");
    assert_eq!(
        d.check(1, 7, b"b"),
        Err(wire::KERR_ALREADY_EXISTS),
        "a conflicting reuse"
    );
    assert_eq!(d.check(2, 7, b"a"), Ok(None), "another caller");
    for n in 0..Dedup::MAX as u64 {
        d.record(1, 100 + n, b"x", 1);
    }
    assert_eq!(d.check(1, 7, b"b"), Ok(None), "the oldest is forgotten");
}

// --- owner work -----------------------------------------------------------------------------------------------------

const S: u32 = 7;

fn sid(key: &str) -> u32 {
    apps::sid(key).unwrap()
}

/// Clock registered on session [`S`] for both kinds of work, offering its alarms.
fn table() -> Jobs {
    let mut j = Jobs::default();
    j.set_offered("clock", scope::CLOCK | scope::PREFERENCES);
    j.register("clock", sid("clock"), S, 20, &[1, 3], (0, UTC))
        .unwrap();
    j
}

fn done(st: u8, committed: u32, result: i32) -> OwnerReport {
    OwnerReport {
        state: st,
        committed_scope: committed,
        result,
        ..OwnerReport::default()
    }
}

#[test]
fn a_confirmed_clear_end_to_end_and_a_duplicate_ack() {
    let mut j = table();
    let t = j
        .submit(ipc::ACTION_CLEAR, "clock", scope::CLOCK, 0)
        .unwrap();
    assert_eq!(j.jobs().next().unwrap().state, state::QUEUED);
    assert_eq!(
        j.wakes(),
        vec![("clock", S, AppEvent::ClearTicket { ticket: t })]
    );
    assert!(j.wakes().is_empty(), "woken once");
    let w = j.claim("clock", sid("clock"), S, t).unwrap();
    assert_eq!(
        w,
        OwnerWork {
            work: wire::WORK_CLEAR,
            subject: AppRef::Native("clock"),
            scope: scope::CLOCK
        }
    );
    assert_eq!(
        j.claim("clock", sid("clock"), S, t),
        Ok(w),
        "claiming again on the session"
    );
    let r = done(state::COMMITTED, scope::CLOCK, 0);
    assert_eq!(
        j.finish("clock", sid("clock"), S, t, r.clone(), UTC),
        Ok(true)
    );
    assert_eq!(
        j.finish("clock", sid("clock"), S, t, r, UTC + 9),
        Ok(false),
        "the same ACK again"
    );
    assert_eq!(
        j.finish("clock", sid("clock"), S, t, done(state::FAILED, 0, -1), UTC),
        Err(wire::KERR_ALREADY_EXISTS)
    );
    let jobs = wire::decode_jobs(&j.records()).unwrap();
    assert_eq!(
        jobs,
        vec![Job {
            ticket: t,
            action: 6,
            state: state::COMMITTED,
            subject: Some(AppRef::Native("clock")),
            requested_scope: scope::CLOCK,
            committed_scope: scope::CLOCK,
            result: 0,
            finished_utc: UTC,
        }]
    );
}

#[test]
fn a_partial_clear_and_reports_that_dont_fit() {
    let mut j = table();
    let asked = scope::CLOCK | scope::PREFERENCES;
    let t = j.submit(ipc::ACTION_CLEAR, "clock", asked, 0).unwrap();
    j.claim("clock", sid("clock"), S, t).unwrap();
    let f = |j: &mut Jobs, r| j.finish("clock", sid("clock"), S, t, r, UTC);
    assert_eq!(
        f(&mut j, done(state::COMMITTED, scope::CLOCK, 0)),
        Err(kerr::ARGUMENT),
        "commit of less"
    );
    assert_eq!(
        f(&mut j, done(state::PARTIAL, scope::SMS, 0)),
        Err(kerr::ARGUMENT),
        "more than asked"
    );
    assert_eq!(
        f(&mut j, done(state::FAILED, 0, 0)),
        Err(kerr::ARGUMENT),
        "a failure without an error"
    );
    assert_eq!(
        f(&mut j, done(state::COMMITTED, asked, -1)),
        Err(kerr::ARGUMENT),
        "a success with one"
    );
    assert_eq!(f(&mut j, done(state::PARTIAL, scope::CLOCK, -1)), Ok(true));
    let job = *j.jobs().next().unwrap();
    assert_eq!(
        (job.state, job.committed_scope, job.result),
        (state::PARTIAL, scope::CLOCK, -1)
    );
}

#[test]
fn only_the_registered_owner_on_its_session() {
    let mut j = Jobs::default();
    let c = sid("clock");
    // who may register
    assert_eq!(
        j.register("clock", c, S, 19, &[1, 1], (0, UTC)),
        Err(kerr::PERMISSION_DENIED),
        "a 1.19 client"
    );
    assert_eq!(
        j.register("clock", c + 1, S, 20, &[1, 1], (0, UTC)),
        Err(kerr::PERMISSION_DENIED),
        "another SID"
    );
    assert_eq!(
        j.register("kit", sid("kit"), S, 20, &[1, 1], (0, UTC)),
        Err(kerr::PERMISSION_DENIED),
        "the shared SID"
    );
    assert_eq!(
        j.register(apps::LEGACY, 0, S, 20, &[1, 1], (0, UTC)),
        Err(kerr::PERMISSION_DENIED)
    );
    assert_eq!(
        j.register("clock", c, S, 20, &[1, 0], (0, UTC)),
        Err(kerr::ARGUMENT)
    );
    let mut j = table();
    let t = j.submit(ipc::ACTION_MEASURE_APP, "clock", 0, 0).unwrap();
    assert_eq!(
        j.claim("clock", c, S + 1, t),
        Err(kerr::PERMISSION_DENIED),
        "a stale session"
    );
    assert_eq!(
        j.claim("clock", c + 1, S, t),
        Err(kerr::PERMISSION_DENIED),
        "a wrong SID"
    );
    assert_eq!(
        j.claim("calendar", sid("calendar"), S, t),
        Err(kerr::PERMISSION_DENIED),
        "not registered"
    );
    j.register("calendar", sid("calendar"), 9, 20, &[1, 1], (0, UTC))
        .unwrap();
    assert_eq!(
        j.claim("calendar", sid("calendar"), 9, t),
        Err(kerr::PERMISSION_DENIED),
        "another's ticket"
    );
    assert_eq!(j.claim("clock", c, S, 999), Err(kerr::NOT_FOUND));
    // a new session replaces the old registration
    j.register("clock", c, 11, 20, &[1, 1], (0, UTC)).unwrap();
    assert_eq!(j.claim("clock", c, S, t), Err(kerr::PERMISSION_DENIED));
    j.claim("clock", c, 11, t).unwrap();
    // finishing needs the claim on this session
    let t2 = j.submit(ipc::ACTION_MEASURE_APP, "calendar", 0, 0).unwrap();
    assert_eq!(
        j.finish(
            "calendar",
            sid("calendar"),
            9,
            t2,
            done(state::COMMITTED, 0, 0),
            UTC
        ),
        Err(kerr::ARGUMENT),
        "not claimed"
    );
    // registered without the clear capability: a clear is refused at once
    j.register(
        "clock",
        c,
        11,
        20,
        &[1, wire::capability::MEASURE],
        (0, UTC),
    )
    .unwrap();
    assert_eq!(
        j.submit(ipc::ACTION_CLEAR, "clock", scope::CLOCK, 0),
        Err(kerr::NOT_SUPPORTED)
    );
}

#[test]
fn submit_rules() {
    let mut j = table();
    assert_eq!(
        j.submit(ipc::ACTION_CLEAR, "clock", scope::SMS, 0),
        Err(kerr::NOT_SUPPORTED),
        "not offered"
    );
    assert_eq!(
        j.submit(ipc::ACTION_CLEAR, "clock", 0, 0),
        Err(kerr::NOT_SUPPORTED)
    );
    assert_eq!(
        j.submit(ipc::ACTION_CLEAR, "files", scope::CACHE, 0),
        Err(kerr::NOT_SUPPORTED),
        "no INFO 8 yet"
    );
    assert_eq!(
        j.submit(ipc::ACTION_STOP, "clock", 0, 0),
        Err(kerr::ARGUMENT)
    );
    // concurrent measurements of one app share a ticket
    let a = j.submit(ipc::ACTION_MEASURE_APP, "clock", 0, 0).unwrap();
    assert_eq!(j.submit(ipc::ACTION_MEASURE_APP, "clock", 0, 5), Ok(a));
    let b = j.submit(ipc::ACTION_MEASURE_APP, "files", 0, 0).unwrap();
    assert_ne!(a, b);
    assert!(a > 0 && b > 0);
    for _ in 2..jobs::UNFINISHED_MAX {
        j.submit(ipc::ACTION_CLEAR, "clock", scope::CLOCK, 0)
            .unwrap();
    }
    assert_eq!(
        j.submit(ipc::ACTION_CLEAR, "clock", scope::CLOCK, 0),
        Err(kerr::OVERFLOW)
    );
}

#[test]
fn a_cold_owner_is_started_and_an_absent_one_expires() {
    let mut j = Jobs::default();
    let t = j.submit(ipc::ACTION_MEASURE_APP, "files", 0, 1000).unwrap();
    assert!(j.wakes().is_empty(), "no owner yet");
    assert_eq!(j.waiting_owners(), vec!["files"]);
    j.register("files", sid("files"), S, 20, &[1, 1], (0, UTC))
        .unwrap();
    assert!(j.waiting_owners().is_empty());
    assert_eq!(
        j.wakes(),
        vec![("files", S, AppEvent::OwnerWork { ticket: t })]
    );
    // another, whose app never registers (a 1.19 app)
    let u = j
        .submit(ipc::ACTION_MEASURE_APP, "browser", 0, 1000)
        .unwrap();
    assert!(!j.expire(1000 + jobs::OWNER_WAIT_MS, UTC));
    assert!(j.expire(1001 + jobs::OWNER_WAIT_MS, UTC));
    let job = |j: &Jobs, t| *j.jobs().find(|x| x.ticket == t).unwrap();
    assert_eq!(
        (job(&j, u).state, job(&j, u).result),
        (state::FAILED, kerr::NOT_SUPPORTED)
    );
    // the woken one that never claimed timed out
    assert_eq!(
        (job(&j, t).state, job(&j, t).result),
        (state::FAILED, jobs::KERR_TIMED_OUT)
    );
}

#[test]
fn owner_exit() {
    let mut j = table();
    let c = sid("clock");
    // claimed, then the owner went: interrupted, and a later report can't revive it
    let t = j
        .submit(ipc::ACTION_CLEAR, "clock", scope::CLOCK, 0)
        .unwrap();
    j.wakes();
    j.claim("clock", c, S, t).unwrap();
    assert!(j.session_gone(S, 10, UTC));
    let job = *j.jobs().next().unwrap();
    assert_eq!(
        (job.state, job.result),
        (state::INTERRUPTED, wire::KERR_DIED)
    );
    j.register("clock", c, 8, 20, &[1, 3], (0, UTC)).unwrap();
    assert_eq!(
        j.finish(
            "clock",
            c,
            8,
            t,
            done(state::COMMITTED, scope::CLOCK, 0),
            UTC
        ),
        Err(wire::KERR_ALREADY_EXISTS)
    );
    // only woken: it is woken again once the owner is back
    let t2 = j.submit(ipc::ACTION_MEASURE_APP, "clock", 0, 0).unwrap();
    assert_eq!(j.wakes().len(), 1);
    assert!(!j.session_gone(8, 20, UTC), "no job changed");
    assert!(j.wakes().is_empty(), "no owner now");
    j.register("clock", c, 9, 20, &[1, 3], (0, UTC)).unwrap();
    assert_eq!(
        j.wakes(),
        vec![("clock", 9, AppEvent::OwnerWork { ticket: t2 })]
    );
}

#[test]
fn a_restart_between_commit_and_ack() {
    let mut j = table();
    let c = sid("clock");
    let done_t = j.submit(ipc::ACTION_MEASURE_APP, "clock", 0, 0).unwrap();
    j.claim("clock", c, S, done_t).unwrap();
    j.finish("clock", c, S, done_t, done(state::COMMITTED, 0, 0), UTC)
        .unwrap();
    // the owner committed the clear, but the phone restarted before its report
    let t = j
        .submit(ipc::ACTION_CLEAR, "clock", scope::CLOCK, 0)
        .unwrap();
    j.claim("clock", c, S, t).unwrap();
    let queued = j.submit(ipc::ACTION_MEASURE_APP, "files", 0, 0).unwrap();
    let saved = j.encode();
    let mut after = Jobs::restore(&saved, UTC + 60).unwrap();
    let job = |j: &Jobs, t| *j.jobs().find(|x| x.ticket == t).unwrap();
    assert_eq!(
        job(&after, done_t).state,
        state::COMMITTED,
        "a receipt stays"
    );
    for x in [t, queued] {
        let r = job(&after, x);
        assert_eq!(
            (r.state, r.result, r.finished_utc),
            (state::INTERRUPTED, wire::KERR_DIED, UTC + 60)
        );
    }
    // nothing resumes, a late report is refused, and the new tickets don't reuse the old
    assert!(after.wakes().is_empty());
    after
        .register("clock", c, 3, 20, &[1, 3], (0, UTC))
        .unwrap();
    assert!(after.wakes().is_empty());
    assert_eq!(
        after.finish(
            "clock",
            c,
            3,
            t,
            done(state::COMMITTED, scope::CLOCK, 0),
            UTC
        ),
        Err(wire::KERR_ALREADY_EXISTS)
    );
    let n = after
        .submit(ipc::ACTION_MEASURE_APP, "clock", 0, 0)
        .unwrap();
    assert!(n > queued);
    // damaged files
    assert_eq!(Jobs::restore(&saved[..saved.len() - 1], UTC), None);
    assert_eq!(Jobs::restore(&[saved.clone(), vec![0]].concat(), UTC), None);
    assert_eq!(Jobs::restore(&[], UTC), None);
}

#[test]
fn receipts_are_bounded() {
    let mut j = table();
    let c = sid("clock");
    for _ in 0..jobs::RECEIPTS_MAX + 5 {
        let t = j.submit(ipc::ACTION_MEASURE_APP, "clock", 0, 0).unwrap();
        j.claim("clock", c, S, t).unwrap();
        j.finish("clock", c, S, t, done(state::COMMITTED, 0, 0), UTC)
            .unwrap();
    }
    let pending = j.submit(ipc::ACTION_MEASURE_APP, "clock", 0, 0).unwrap();
    let tickets: Vec<u32> = j.jobs().map(|x| x.ticket).collect();
    assert_eq!(tickets.len(), jobs::RECEIPTS_MAX + 1);
    assert_eq!(tickets[0], 6, "the oldest went");
    assert_eq!(*tickets.last().unwrap(), pending);
    assert!(j.measured("clock").is_some());
    // INFO 13 fits its pages
    let records = j.records();
    assert!(records.iter().all(|r| r.len() <= Job::MAX));
    assert!(wire::page(13, 1, &records, 0, 0, wire::PAGE_MAX).is_ok());
}

// --- review fixes (w1) -----------------------------------------------------------------------------------------------

#[test]
fn events_1_20_words_are_exact() {
    for w in [
        [27, 13, 0, 0],          // a zero revision
        [27, 13, 1, 1],          // a reserved word
        [29, 0, 0, 0],           // ticket 0
        [29, 0x8000_0000, 0, 0], // past i32::MAX
        [29, 1, 1, 0],           // a reserved word
        [28, 0, 1, 0],           // not the exact 1.17 vector
        [28, 0, 0, 1],
        [28, 0x8000_0000, 0, 0], // a ticket past i32::MAX
        [28, 5, 0, 9],
    ] {
        assert_eq!(AppEvent::decode(&ev(w)), None, "{w:?}");
    }
    assert_eq!(
        AppEvent::decode(&ev([28, 0, 0, 0])),
        Some(AppEvent::ClearData)
    );
    assert_eq!(
        AppEvent::decode(&ev([29, 0x7fff_ffff, 0, 0])),
        Some(AppEvent::OwnerWork {
            ticket: 0x7fff_ffff
        })
    );
}

fn header(count: u16, rev: u32, first: u32, next: u32) -> Vec<u8> {
    let mut b = vec![1, 13];
    b.extend_from_slice(&count.to_le_bytes());
    for w in [rev, first, next] {
        b.extend_from_slice(&w.to_le_bytes());
    }
    b
}

#[test]
fn a_page_claims_no_more_than_its_bytes_hold() {
    // 16 bytes claiming 65535 records: refused before anything is reserved
    assert!(wire::parse_page(&header(u16::MAX, 1, 0, wire::TERMINAL), 13, 0, 0).is_none());
    // two empty records need four bytes
    let mut b = header(2, 1, 0, wire::TERMINAL);
    b.extend_from_slice(&[0, 0]);
    assert!(wire::parse_page(&b, 13, 0, 0).is_none());
    b.extend_from_slice(&[0, 0]);
    assert_eq!(wire::parse_page(&b, 13, 0, 0).unwrap().1.len(), 2);
}

#[test]
fn only_a_first_page_is_empty_and_cursors_dont_overflow() {
    // an empty terminal continuation would publish a cut snapshot as whole
    assert!(wire::parse_page(&header(0, 1, 2, wire::TERMINAL), 13, 2, 1).is_none());
    let mut t = Traversal::new(13);
    let p0 = wire::page(13, 1, &recs(3, 1), 0, 0, wire::HEADER_LEN + 3).unwrap();
    assert_eq!(t.feed(&p0), Some(false));
    assert_eq!(t.feed(&header(0, 1, 1, wire::TERMINAL)), None);
    // the cursor arithmetic: first + count past u32, or landing on the terminal value
    let mut b = header(1, 1, u32::MAX - 1, wire::TERMINAL);
    b.extend_from_slice(&[0, 0]);
    assert!(wire::parse_page(&b, 13, u32::MAX - 1, 1).is_none());
    let mut b = header(2, 1, u32::MAX - 1, wire::TERMINAL);
    b.extend_from_slice(&[0, 0, 0, 0]);
    assert!(wire::parse_page(&b, 13, u32::MAX - 1, 1).is_none());
}

#[test]
fn contradictory_receipts_are_refused() {
    let measure = Job {
        action: ipc::ACTION_MEASURE_APP as u8,
        requested_scope: 0,
        committed_scope: 0,
        result: 0,
        ..job()
    };
    let bad = [
        Job {
            state: state::FAILED,
            ..measure
        }, // failed without an error
        Job {
            state: state::COMMITTED,
            result: -1,
            ..measure
        }, // committed with one
        Job {
            state: state::PARTIAL,
            result: 1,
            ..measure
        },
        Job {
            state: state::COMMITTED,
            result: 0,
            ..job()
        }, // committed less than asked
        Job {
            state: state::QUEUED,
            finished_utc: 0,
            result: -1,
            ..measure
        },
        Job {
            state: state::QUEUED,
            committed_scope: 0,
            ..job()
        }, // finished time on a queued job
        Job {
            state: state::RUNNING,
            committed_scope: scope::CLOCK,
            finished_utc: 0,
            ..job()
        },
    ];
    for j in bad {
        assert_eq!(Job::decode(&j.encode()), None, "{j:?}");
        // and a saved table holding it doesn't restore
        let rec = j.encode();
        let mut file = vec![2, 9, 0, 0, 0, 1, 0];
        file.extend_from_slice(&(rec.len() as u16).to_le_bytes());
        file.extend_from_slice(&rec);
        file.extend_from_slice(&[0; 8]);
        assert_eq!(Jobs::restore(&file, UTC), None, "{j:?}");
    }
    let good = Job {
        state: state::COMMITTED,
        ..measure
    };
    assert_eq!(Job::decode(&good.encode()), Some(good));
    let mut file = vec![2, 9, 0, 0, 0, 1, 0];
    let rec = good.encode();
    file.extend_from_slice(&(rec.len() as u16).to_le_bytes());
    file.extend_from_slice(&rec);
    file.extend_from_slice(&[0; 8]);
    assert!(Jobs::restore(&file, UTC).is_some());
    // an unfinished job can't carry a report's fingerprint
    let q = Job {
        state: state::QUEUED,
        result: 0,
        finished_utc: 0,
        ..measure
    }
    .encode();
    let mut file = vec![2, 9, 0, 0, 0, 1, 0];
    file.extend_from_slice(&(q.len() as u16).to_le_bytes());
    file.extend_from_slice(&q);
    file.extend_from_slice(&[1; 8]);
    assert_eq!(Jobs::restore(&file, UTC), None);
}

#[test]
fn registering_again_rearms_wakes_and_interrupts_old_claims() {
    let mut j = table();
    let c = sid("clock");
    let clear = j
        .submit(ipc::ACTION_CLEAR, "clock", scope::CLOCK, 0)
        .unwrap();
    let measure = j.submit(ipc::ACTION_MEASURE_APP, "clock", 0, 0).unwrap();
    assert_eq!(j.wakes().len(), 2);
    j.claim("clock", c, S, measure).unwrap();
    // the app connects again (a new session) and registers: its woken clear is woken again there, the claim on the
    // old session is interrupted
    j.register("clock", c, 8, 20, &[1, 3], (100, UTC)).unwrap();
    assert_eq!(
        j.wakes(),
        vec![("clock", 8, AppEvent::ClearTicket { ticket: clear })]
    );
    let m = *j.jobs().find(|x| x.ticket == measure).unwrap();
    assert_eq!((m.state, m.result), (state::INTERRUPTED, wire::KERR_DIED));
    // the same session registering again keeps its claim
    j.claim("clock", c, 8, clear).unwrap();
    j.register("clock", c, 8, 20, &[1, 3], (100, UTC)).unwrap();
    assert_eq!(
        j.jobs().find(|x| x.ticket == clear).unwrap().state,
        state::RUNNING
    );
    // but without the capability now it can't finish
    j.register(
        "clock",
        c,
        8,
        20,
        &[1, wire::capability::MEASURE],
        (100, UTC),
    )
    .unwrap();
    assert_eq!(
        j.finish(
            "clock",
            c,
            8,
            clear,
            done(state::COMMITTED, scope::CLOCK, 0),
            UTC
        ),
        Err(kerr::PERMISSION_DENIED)
    );
}

#[test]
fn the_same_report_after_a_restart_is_recognised() {
    let mut j = table();
    let c = sid("clock");
    let t = j
        .submit(ipc::ACTION_CLEAR, "clock", scope::CLOCK, 0)
        .unwrap();
    j.claim("clock", c, S, t).unwrap();
    let r = done(state::COMMITTED, scope::CLOCK, 0);
    j.finish("clock", c, S, t, r.clone(), UTC).unwrap();
    // the ACK was lost in the restart: the owner sends the same report again
    let mut after = Jobs::restore(&j.encode(), UTC + 60).unwrap();
    after
        .register("clock", c, 3, 20, &[1, 3], (0, UTC))
        .unwrap();
    assert_eq!(after.finish("clock", c, 3, t, r, UTC + 61), Ok(false));
    assert_eq!(
        after.finish("clock", c, 3, t, done(state::PARTIAL, 0, -1), UTC),
        Err(wire::KERR_ALREADY_EXISTS)
    );
    assert_eq!(
        after.jobs().next().unwrap().finished_utc,
        UTC,
        "the receipt unchanged"
    );
}

#[test]
fn receipts_are_kept_in_the_order_jobs_finished() {
    let mut j = table();
    let c = sid("clock");
    let f = sid("files");
    j.register("files", f, 9, 20, &[1, 1], (0, UTC)).unwrap();
    // a slow clear, then more than RECEIPTS_MAX quick measurements finish before it
    let slow = j
        .submit(ipc::ACTION_CLEAR, "clock", scope::CLOCK, 0)
        .unwrap();
    j.claim("clock", c, S, slow).unwrap();
    for _ in 0..jobs::RECEIPTS_MAX + 3 {
        let t = j.submit(ipc::ACTION_MEASURE_APP, "files", 0, 0).unwrap();
        j.claim("files", f, 9, t).unwrap();
        j.finish("files", f, 9, t, done(state::COMMITTED, 0, 0), UTC)
            .unwrap();
    }
    assert_eq!(j.jobs().next().unwrap().ticket, slow, "unfinished: kept");
    j.finish(
        "clock",
        c,
        S,
        slow,
        done(state::COMMITTED, scope::CLOCK, 0),
        UTC,
    )
    .unwrap();
    let tickets: Vec<u32> = j.jobs().map(|x| x.ticket).collect();
    assert_eq!(tickets.len(), jobs::RECEIPTS_MAX);
    assert_eq!(
        *tickets.last().unwrap(),
        slow,
        "the newest receipt is the last to finish"
    );
    // a restart keeps that order, and the jobs it interrupts finish after it
    let pending = j.submit(ipc::ACTION_MEASURE_APP, "files", 0, 0).unwrap();
    let after = Jobs::restore(&j.encode(), UTC + 1).unwrap();
    let order: Vec<u32> = after.jobs().map(|x| x.ticket).collect();
    assert_eq!(&order[order.len() - 2..], &[slow, pending]);
    assert_eq!(order.len(), jobs::RECEIPTS_MAX);
}

#[test]
fn every_unregistered_owner_is_listed_and_a_wrapped_clock_restarts_the_wait() {
    let mut j = Jobs::default();
    // the browser (a 1.19 app, running, never registers) doesn't hide Files behind it
    let b = j
        .submit(ipc::ACTION_MEASURE_APP, "browser", 0, 5_000_000)
        .unwrap();
    j.submit(ipc::ACTION_MEASURE_APP, "files", 0, 5_000_000)
        .unwrap();
    j.submit(ipc::ACTION_MEASURE_APP, "browser", 0, 5_000_000)
        .unwrap();
    assert_eq!(j.waiting_owners(), vec!["browser", "files"]);
    // the millisecond counter wrapped: no expiry, the wait starts again
    assert!(!j.expire(10, UTC));
    assert!(!j.expire(10 + jobs::OWNER_WAIT_MS, UTC));
    assert!(j.expire(11 + jobs::OWNER_WAIT_MS, UTC));
    assert_eq!(
        j.jobs().find(|x| x.ticket == b).unwrap().state,
        state::FAILED
    );
}

#[test]
fn a_claim_is_checked_without_taking_the_job() {
    let mut j = table();
    let c = sid("clock");
    let t = j.submit(ipc::ACTION_MEASURE_APP, "clock", 0, 0).unwrap();
    let before = j.clone();
    let w = j.claimable("clock", c, S, t).unwrap();
    assert_eq!(j, before, "nothing changed");
    assert_eq!(
        j.claimable("clock", c, S + 1, t),
        Err(kerr::PERMISSION_DENIED)
    );
    assert_eq!(j.claim("clock", c, S, t), Ok(w));
    assert_eq!(j.jobs().next().unwrap().state, state::RUNNING);
}

#[test]
fn report_fingerprints_differ_and_are_never_zero() {
    let a = done(state::COMMITTED, 0, 0);
    let b = done(state::COMMITTED, 0, 0);
    assert_eq!(jobs::fingerprint(&a), jobs::fingerprint(&b));
    assert_ne!(
        jobs::fingerprint(&a),
        jobs::fingerprint(&done(state::PARTIAL, 0, 0))
    );
    assert_ne!(jobs::fingerprint(&a), 0);
}
