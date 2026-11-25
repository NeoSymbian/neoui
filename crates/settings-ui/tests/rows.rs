// SPDX-License-Identifier: GPL-3.0-only
//! The search table (`neoui_proto::settings_app::rows::ROWS`) and the pages can't drift: every row title the
//! `.slint` pages draw is in the table, and every table title is on its pages. Licences (not settings) is left out.
use neoui_proto::settings_app::rows::ROWS;
use std::collections::BTreeSet;
use std::path::Path;

/// The literal `title: "…"` of every row the pages draw.
fn page_titles() -> BTreeSet<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/pages");
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap() == "licences.slint" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("title: \"") {
                out.insert(rest.split('"').next().unwrap().to_string());
            }
        }
    }
    out
}

#[test]
fn the_search_table_matches_the_pages() {
    let pages = page_titles();
    let table: BTreeSet<String> = ROWS.iter().map(|r| r.title.to_string()).collect();
    let missing_in_table: Vec<_> = pages.difference(&table).collect();
    let missing_on_pages: Vec<_> = table.difference(&pages).collect();
    assert!(
        missing_in_table.is_empty(),
        "rows on the pages but not in settings_app::rows: {missing_in_table:?}"
    );
    assert!(
        missing_on_pages.is_empty(),
        "rows in settings_app::rows but on no page: {missing_on_pages:?}"
    );
}
