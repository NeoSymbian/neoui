// SPDX-License-Identifier: Apache-2.0
//! D4 (docs neoui-shell/settings-data.md): the shell's own measurement of an app that isn't an owner, a clear's report
//! as the new size, and what Settings says from an INFO 8 record (the set2-apps and set2-app-browser strings).

use neoui_proto::ipc;
use neoui_proto::settings::kerr;
use neoui_proto::settings_app::app_detail::{
    identity, list_subtitle, size_text, storage_used, with_installed,
};
use neoui_proto::settings_app::jobs::Jobs;
use neoui_proto::settings_app::wire::{
    AppDetail, AppRef, DriveUsage, OwnerReport, coverage, scope, state,
};

const UTC: i64 = 1_700_000_000;
const MB: u64 = 1 << 20;

fn drive(drive: u8, data: Option<u64>, cache: Option<u64>, logs: Option<u64>) -> DriveUsage {
    DriveUsage {
        drive,
        installed: None,
        data,
        cache,
        logs,
    }
}

fn detail(key: &'static str, cov: u8, drives: Vec<DriveUsage>) -> AppDetail {
    AppDetail {
        app: AppRef::Native(key),
        caption: String::new(),
        exe: String::new(),
        version: String::new(),
        identity_source: 2,
        sid: 0,
        cap_source: 0,
        caps: 0,
        coverage: cov,
        measured_utc: UTC,
        clear_scope: 0,
        drives: with_installed(&drives, Some((25, 3 * MB))),
    }
}

#[test]
fn sizes_have_one_decimal_from_mb_up() {
    for (b, want) in [
        (0, "0 B"),
        (1023, "1023 B"),
        (16 * 1024, "16 KB"),
        (64 * 1024 + 100, "64 KB"),
        (1024 * 1024 - 1, "1 MB"),
        (2 * MB + MB * 4 / 10, "2.4 MB"),
        (48 * MB + MB / 5, "48.2 MB"),
        (2 * MB, "2 MB"),
        (MB * 1535 / 1000 * 1024, "1.5 GB"),
    ] {
        assert_eq!(size_text(b), want, "{b}");
    }
}

#[test]
fn the_boards_list_subtitles() {
    let d = |key, cov, drives| detail(key, cov, drives);
    let mb = |x: f64| (x * MB as f64) as u64;
    assert_eq!(
        list_subtitle(
            "browser",
            Some(&d(
                "browser",
                coverage::COMPLETE,
                vec![drive(2, Some(mb(12.6)), Some(mb(35.6)), None)]
            ))
        ),
        "ROM · 48.2 MB data on C:"
    );
    assert_eq!(
        list_subtitle(
            "devtools",
            Some(&d(
                "devtools",
                coverage::COMPLETE,
                vec![
                    drive(2, None, None, Some(0)),
                    drive(4, None, None, Some(mb(1.1)))
                ]
            ))
        ),
        "ROM · 1.1 MB logs on E:"
    );
    assert_eq!(
        list_subtitle(
            "terminal",
            Some(&d(
                "terminal",
                coverage::COMPLETE,
                vec![drive(2, Some(0), None, None)]
            ))
        ),
        "ROM · no data"
    );
    assert_eq!(
        list_subtitle(
            "phone",
            Some(&d(
                "phone",
                coverage::PARTIAL,
                vec![drive(2, Some(mb(2.4)), None, None)]
            ))
        ),
        "ROM · at least 2.4 MB data on C:"
    );
    // a partial zero says nothing about the app's data (Clock's live in DTSRV's folder)
    assert_eq!(
        list_subtitle(
            "clock",
            Some(&d(
                "clock",
                coverage::PARTIAL,
                vec![drive(2, Some(0), None, None)]
            ))
        ),
        "ROM · size unavailable"
    );
    assert_eq!(list_subtitle("files", None), "ROM · not measured yet");
    assert_eq!(
        list_subtitle("files", Some(&d("files", coverage::MEASURING, vec![]))),
        "ROM · counting…"
    );
    assert_eq!(
        list_subtitle("messages", None),
        "ROM · SMS kept by the modem server"
    );
    // installed on C: (drive=c in exes.txt)
    let mut c = d("files", coverage::UNKNOWN, vec![]);
    c.drives = with_installed(&[], Some((2, MB)));
    assert_eq!(list_subtitle("files", Some(&c)), "C: · not measured yet");
}

#[test]
fn the_boards_detail_rows() {
    let mut b = detail(
        "browser",
        coverage::COMPLETE,
        vec![drive(
            2,
            Some(12 * MB + MB * 6 / 10),
            Some(35 * MB + MB * 6 / 10),
            None,
        )],
    );
    b.exe = "NEOWEB.EXE".into();
    assert_eq!(identity(&b), "NEOWEB.EXE");
    b.version = "Servo 0.8.0".into();
    assert_eq!(identity(&b), "NEOWEB.EXE · Servo 0.8.0");
    assert_eq!(
        storage_used(&b),
        ("Data 12.6 MB · cache 35.6 MB".into(), "48.2 MB".into())
    );
    b.coverage = coverage::PARTIAL;
    assert_eq!(storage_used(&b).1, "At least 48.2 MB");
    let logs = detail(
        "devtools",
        coverage::COMPLETE,
        vec![
            drive(2, None, None, Some(200 * 1024)),
            drive(4, None, None, Some(MB)),
        ],
    );
    assert_eq!(
        storage_used(&logs),
        ("Logs 200 KB · logs 1 MB on E:".into(), "1.2 MB".into())
    );
    for (cov, want) in [
        (coverage::UNKNOWN, "Not measured yet"),
        (coverage::MEASURING, "Counting…"),
        (coverage::FAILED, "Size unavailable"),
    ] {
        assert_eq!(storage_used(&detail("x", cov, vec![])).0, want);
    }
    let empty = detail(
        "terminal",
        coverage::COMPLETE,
        vec![drive(2, Some(0), None, None)],
    );
    assert_eq!(storage_used(&empty), ("No data".into(), "0 B".into()));
}

#[test]
fn installed_bytes_are_the_shells() {
    let owner = [
        DriveUsage {
            drive: 25,
            installed: Some(1),
            data: Some(5),
            ..DriveUsage::default()
        },
        drive(2, Some(7), None, None),
    ];
    let m = with_installed(&owner, Some((25, 99)));
    assert_eq!(m.len(), 2);
    assert_eq!((m[0].drive, m[0].installed, m[0].data), (2, None, Some(7)));
    assert_eq!(
        (m[1].drive, m[1].installed, m[1].data),
        (25, Some(99), Some(5))
    );
    assert_eq!(with_installed(&owner, None)[1].installed, None);
}

fn measured(cov: u8, drives: Vec<DriveUsage>) -> OwnerReport {
    OwnerReport {
        state: if cov == coverage::COMPLETE {
            state::COMMITTED
        } else {
            state::PARTIAL
        },
        coverage: cov,
        measured_utc: UTC,
        drives,
        ..OwnerReport::default()
    }
}

#[test]
fn the_shell_measures_an_app_that_isnt_an_owner() {
    let mut j = Jobs::default();
    let t = j.submit(ipc::ACTION_MEASURE_APP, "files", 0, 0).unwrap();
    let r = measured(coverage::PARTIAL, vec![drive(2, Some(10), None, None)]);
    assert_eq!(j.finish_here("files", t, r.clone(), UTC), Ok(()));
    assert_eq!(j.measured("files"), Some(&r));
    assert_eq!(j.jobs().next().unwrap().state, state::PARTIAL);
    assert_eq!(
        j.finish_here("files", t, r.clone(), UTC),
        Err(kerr::NOT_FOUND),
        "finished"
    );
    assert!(j.waiting_owners().is_empty(), "nobody to start");
    // never a clear, never another app's ticket, never a report that doesn't fit
    j.set_offered("devtools", scope::LOGS);
    let c = j
        .submit(ipc::ACTION_CLEAR, "devtools", scope::LOGS, 0)
        .unwrap();
    assert_eq!(
        j.finish_here("devtools", c, r.clone(), UTC),
        Err(kerr::NOT_FOUND)
    );
    let m = j.submit(ipc::ACTION_MEASURE_APP, "clock", 0, 0).unwrap();
    assert_eq!(
        j.finish_here("files", m, r.clone(), UTC),
        Err(kerr::PERMISSION_DENIED)
    );
    let mut bad = r;
    bad.committed_scope = scope::LOGS;
    assert_eq!(j.finish_here("clock", m, bad, UTC), Err(kerr::ARGUMENT));
}

#[test]
fn a_clears_report_is_the_size_after_it() {
    let mut j = Jobs::default();
    let sid = neoui_proto::apps::sid("devtools").unwrap();
    j.register("devtools", sid, 3, 20, &[1, 3], (0, UTC))
        .unwrap();
    j.set_offered("devtools", scope::LOGS);
    let t = j
        .submit(ipc::ACTION_CLEAR, "devtools", scope::LOGS, 0)
        .unwrap();
    j.claim("devtools", sid, 3, t).unwrap();
    let mut r = measured(coverage::COMPLETE, vec![drive(4, None, None, Some(0))]);
    r.committed_scope = scope::LOGS;
    assert_eq!(j.finish("devtools", sid, 3, t, r.clone(), UTC), Ok(true));
    assert_eq!(j.measured("devtools"), Some(&r));
    // a report without a coverage leaves the last measurement alone
    let t = j
        .submit(ipc::ACTION_CLEAR, "devtools", scope::LOGS, 0)
        .unwrap();
    j.claim("devtools", sid, 3, t).unwrap();
    let f = OwnerReport {
        state: state::FAILED,
        result: -14,
        ..OwnerReport::default()
    };
    j.finish("devtools", sid, 3, t, f, UTC).unwrap();
    assert_eq!(j.measured("devtools"), Some(&r));
}
