<!-- SPDX-License-Identifier: Apache-2.0 -->
# IANA tzdata (vendored subset)

The region source files of the IANA time zone database, release **2025b** (`version`), unchanged:
`africa antarctica asia australasia europe northamerica southamerica etcetera backward`. The database is in the public
domain (`LICENSE`). Fetched once from https://data.iana.org/time-zones/releases/tzdata2025b.tar.gz
(SHA-256 `11810413345fc7805017e27ea9fa4885fd74cd61b2911711ad038f5d28d71474`); nothing fetches it at build time or in CI.

`tools/gen_zones.py` compiles it with `zic` and writes `crates/neoui-proto/src/clock/zones.rs` (the World clock and
Settings → Time zone cities' rules) and `zones_ref.rs` (zic's own 2026–2030 transitions, which the tests check the
device's rule evaluator against). To move to a new release: replace these files, update `version`, run the script.
