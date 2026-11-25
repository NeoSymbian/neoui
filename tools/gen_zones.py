#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate the World clock's city rules from the vendored IANA tzdata (tools/tzdata, docs neoui-shell/settings.md SE1b).

Compiles tools/tzdata with the host's `zic -b fat` (explicit transitions through 2037), takes each city's POSIX TZ
footer as its rule, and writes:
  crates/neoui-proto/src/clock/zones.rs      the release and the cities' rules (CATALOGUE, west to east)
  crates/neoui-proto/src/clock/zones_ref.rs  zic's own transitions 2026-2030 per city (tests only)
Run by hand after changing tools/tzdata; nothing runs it at build time. `--check` (CI) fails if the checked-in files
differ from what tools/tzdata gives.
"""

import calendar
import io
import os
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TZDATA = os.path.join(ROOT, "tools", "tzdata")
OUT = os.path.join(ROOT, "crates", "neoui-proto", "src", "clock")
REGIONS = [
    "africa",
    "antarctica",
    "asia",
    "australasia",
    "europe",
    "northamerica",
    "southamerica",
    "etcetera",
    "backward",
]
YEARS = (2026, 2030)
START = calendar.timegm((YEARS[0], 1, 1, 0, 0, 0))

# The cities (names are saved in `clock.cities` and `time.zone`: never rename one) and their IANA zones, west to east
# by standard offset.
CITIES = [
    ("Pago Pago", "Pacific/Pago_Pago"),
    ("Honolulu", "Pacific/Honolulu"),
    ("Anchorage", "America/Anchorage"),
    ("Los Angeles", "America/Los_Angeles"),
    ("Vancouver", "America/Vancouver"),
    ("Phoenix", "America/Phoenix"),
    ("Denver", "America/Denver"),
    ("Mexico City", "America/Mexico_City"),
    ("Chicago", "America/Chicago"),
    ("Bogotá", "America/Bogota"),
    ("Lima", "America/Lima"),
    ("New York", "America/New_York"),
    ("Toronto", "America/Toronto"),
    ("Havana", "America/Havana"),
    ("Caracas", "America/Caracas"),
    ("Halifax", "America/Halifax"),
    ("Santiago", "America/Santiago"),
    ("St. John's", "America/St_Johns"),
    ("São Paulo", "America/Sao_Paulo"),
    ("Buenos Aires", "America/Argentina/Buenos_Aires"),
    ("Nuuk", "America/Nuuk"),
    ("Praia", "Atlantic/Cape_Verde"),
    ("Azores", "Atlantic/Azores"),
    ("Reykjavik", "Atlantic/Reykjavik"),
    ("London", "Europe/London"),
    ("Dublin", "Europe/Dublin"),
    ("Lisbon", "Europe/Lisbon"),
    ("Lagos", "Africa/Lagos"),
    ("Paris", "Europe/Paris"),
    ("Berlin", "Europe/Berlin"),
    ("Madrid", "Europe/Madrid"),
    ("Rome", "Europe/Rome"),
    ("Amsterdam", "Europe/Amsterdam"),
    ("Stockholm", "Europe/Stockholm"),
    ("Vienna", "Europe/Vienna"),
    ("Warsaw", "Europe/Warsaw"),
    ("Prague", "Europe/Prague"),
    ("Cairo", "Africa/Cairo"),
    ("Johannesburg", "Africa/Johannesburg"),
    ("Athens", "Europe/Athens"),
    ("Bucharest", "Europe/Bucharest"),
    ("Helsinki", "Europe/Helsinki"),
    ("Kyiv", "Europe/Kyiv"),
    ("Beirut", "Asia/Beirut"),
    ("Jerusalem", "Asia/Jerusalem"),
    ("Istanbul", "Europe/Istanbul"),
    ("Moscow", "Europe/Moscow"),
    ("Nairobi", "Africa/Nairobi"),
    ("Riyadh", "Asia/Riyadh"),
    ("Baghdad", "Asia/Baghdad"),
    ("Tehran", "Asia/Tehran"),
    ("Dubai", "Asia/Dubai"),
    ("Yerevan", "Asia/Yerevan"),
    ("Tbilisi", "Asia/Tbilisi"),
    ("Baku", "Asia/Baku"),
    ("Kabul", "Asia/Kabul"),
    ("Karachi", "Asia/Karachi"),
    ("Tashkent", "Asia/Tashkent"),
    ("Almaty", "Asia/Almaty"),
    ("Delhi", "Asia/Kolkata"),
    ("Colombo", "Asia/Colombo"),
    ("Kathmandu", "Asia/Kathmandu"),
    ("Dhaka", "Asia/Dhaka"),
    ("Yangon", "Asia/Yangon"),
    ("Bangkok", "Asia/Bangkok"),
    ("Ho Chi Minh City", "Asia/Ho_Chi_Minh"),
    ("Jakarta", "Asia/Jakarta"),
    ("Singapore", "Asia/Singapore"),
    ("Beijing", "Asia/Shanghai"),
    ("Hong Kong", "Asia/Hong_Kong"),
    ("Taipei", "Asia/Taipei"),
    ("Manila", "Asia/Manila"),
    ("Perth", "Australia/Perth"),
    ("Seoul", "Asia/Seoul"),
    ("Tokyo", "Asia/Tokyo"),
    ("Darwin", "Australia/Darwin"),
    ("Adelaide", "Australia/Adelaide"),
    ("Brisbane", "Australia/Brisbane"),
    ("Sydney", "Australia/Sydney"),
    ("Melbourne", "Australia/Melbourne"),
    ("Lord Howe", "Australia/Lord_Howe"),
    ("Nouméa", "Pacific/Noumea"),
    ("Auckland", "Pacific/Auckland"),
    ("Fiji", "Pacific/Fiji"),
    ("Chatham", "Pacific/Chatham"),
    ("Tongatapu", "Pacific/Tongatapu"),
    ("Kiritimati", "Pacific/Kiritimati"),
]


def tzif(path):
    """The v2 block's transitions [(utc, offset)], the offset before the first (type 0) and the POSIX footer."""
    with open(path, "rb") as f:
        data = f.read()
    assert data[:4] == b"TZif" and data[4:5] in (b"2", b"3", b"4"), path

    def counts(at):
        return struct.unpack(">6I", data[at + 20 : at + 44])

    isut, isstd, leap, timecnt, typecnt, charcnt = counts(0)
    v1 = 44 + timecnt * 5 + typecnt * 6 + charcnt + leap * 8 + isstd + isut
    isut, isstd, leap, timecnt, typecnt, charcnt = counts(v1)
    at = v1 + 44
    times = struct.unpack(f">{timecnt}q", data[at : at + timecnt * 8])
    at += timecnt * 8
    idx = data[at : at + timecnt]
    at += timecnt
    types = [
        struct.unpack(">iBB", data[at + i * 6 : at + i * 6 + 6]) for i in range(typecnt)
    ]
    at += typecnt * 6 + charcnt + leap * 12 + isstd + isut
    footer = data[at:].split(b"\n")[1].decode()
    return [(t, types[i][0]) for t, i in zip(times, idx)], types[0][0], footer


def main():
    if shutil.which("zic") is None:
        sys.exit(
            "zic not found (Debian/Ubuntu: libc-bin; macOS ships it); CI runs this check"
        )
    with open(os.path.join(TZDATA, "version")) as f:
        version = f.read().strip()
    lo = START
    hi = calendar.timegm((YEARS[1] + 1, 1, 1, 0, 0, 0))
    with tempfile.TemporaryDirectory() as out:
        subprocess.check_call(
            ["zic", "-b", "fat", "-d", out] + [os.path.join(TZDATA, r) for r in REGIONS]
        )
        rules, refs, starts = [], [], []
        for name, zone in CITIES:
            transitions, first, footer = tzif(os.path.join(out, zone))
            assert footer, f"{zone}: no POSIX footer"
            rules.append((name, zone, footer))
            # the offset when the window opens, then each transition in it with the offset before it
            prev = first
            for t, off in transitions:
                if t > lo:
                    break
                prev = off
            starts.append((name, prev))
            prev = first
            for t, off in transitions:
                if lo <= t < hi:
                    refs.append((name, t, prev, off))
                prev = off
    files = {}
    for name, write, items in (
        ("zones.rs", write_zones, rules),
        ("zones_ref.rs", write_refs, (refs, starts)),
    ):
        w = io.StringIO()
        write(w, version, items)
        files[name] = w.getvalue()
    if "--check" in sys.argv:
        stale = []
        for name, text in files.items():
            with open(os.path.join(OUT, name), encoding="utf-8") as f:
                if f.read() != text:
                    stale.append(name)
        if stale:
            sys.exit(
                f"{', '.join(stale)}: not what tools/tzdata gives; run tools/gen_zones.py"
            )
        print(f"tzdata {version}: zones.rs and zones_ref.rs are up to date")
        return
    for name, text in files.items():
        with open(os.path.join(OUT, name), "w", encoding="utf-8") as f:
            f.write(text)
    print(
        f"tzdata {version}: {len(rules)} cities, {len(refs)} transitions {YEARS[0]}-{YEARS[1]}"
    )


def write_zones(w, version, rules):
    w.write("// SPDX-License-Identifier: Apache-2.0\n")
    w.write(
        f"// Generated by tools/gen_zones.py from IANA tzdata {version} (tools/tzdata). Do not edit.\n"
    )
    w.write(
        "//! The World clock's cities and their rules (POSIX TZ strings, from zic's footers).\n\n"
    )
    w.write("use super::world::City;\n\n")
    w.write(
        f'/// The IANA release the rules come from.\npub const TZDATA: &str = "{version}";\n\n'
    )
    w.write(
        f"/// The years zones_test checks every city against zic's own transitions.\npub const YEARS_CHECKED: (i32, i32) = ({YEARS[0]}, {YEARS[1]});\n\n"
    )
    w.write(
        "/// The cities offered, west to east by standard offset. Names are saved settings: never rename one.\n"
    )
    w.write("pub static CATALOGUE: &[City] = &[\n")
    for name, zone, rule in rules:
        w.write(
            f'    City {{\n        name: "{name}",\n        tz: "{rule}",\n    }}, // {zone}\n'
        )
    w.write("];\n")


def write_refs(w, version, items):
    refs, starts = items
    w.write("// SPDX-License-Identifier: Apache-2.0\n")
    w.write(
        f"// Generated by tools/gen_zones.py from IANA tzdata {version}. Do not edit.\n"
    )
    w.write(
        f"//! zic's own transitions {YEARS[0]}-{YEARS[1]}: (city, UTC seconds, offset before, offset after).\n\n"
    )
    w.write("pub static TRANSITIONS: &[(&str, i64, i32, i32)] = &[\n")
    for name, t, before, after in refs:
        w.write(f'    ("{name}", {t}, {before}, {after}),\n')
    w.write("];\n\n")
    w.write(
        f"/// Each city's offset at {YEARS[0]}-01-01T00:00Z (UTC seconds {START}), from zic.\n"
    )
    w.write("pub static STARTS: &[(&str, i32)] = &[\n")
    for name, off in starts:
        w.write(f'    ("{name}", {off}),\n')
    w.write("];\n")


if __name__ == "__main__":
    main()
