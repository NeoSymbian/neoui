# /// script
# requires-python = ">=3.10"
# dependencies = ["playwright>=1.48", "pillow>=10"]
# ///
# SPDX-License-Identifier: Apache-2.0
"""board_check -- set each design-canvas board beside the snapshot the app draws, for review.

    uv run tools/board_check.py BOARDS [--snapshots snapshots] [--out OUT] [--only set2-]

BOARDS is a folder of the canvas's `<name>.dc.html` boards (the Design artifact's `project/` folder, fetched with the
Artifact tool's read_file). Each board is drawn in headless Chromium at 360x720 (one board dp = one CSS px), the status
bar cut off and the rest cropped to the snapshot's height, then written beside the snapshot (scaled to 360 dp wide)
as OUT/<name>.png. Boards without a snapshot and snapshots without a board are listed at the end.

There is no pixel threshold: the canvas draws Inter and the app NeoSans (owner, 2026-11-26), so a reviewer compares
layout, weights, colours, icons and copy. First run: `uv run --with playwright playwright install chromium-headless-shell`.
"""

import argparse
import pathlib
import sys

from PIL import Image, ImageDraw
from playwright.sync_api import sync_playwright

# boards whose snapshot has another name (the app's page replaced the board's example)
SNAPSHOT_FOR = {
    "set2-app-browser": "set2-app-detail",
    "sh-shade-quick": "tp-shade",
    # (G2/G3) Open apps in Gestures: the switcher's own snapshot
    "nav-openapps": "switcher-gestures-539x1079",
    # home (D10, home.md Parts 3 and 4)
    "Main": "home-pixel",
    "sh-home-buttons": "home-buttons-539x1079",
    "sh-home-clock-24h": "home-clock-widget-540x1080",
    "sh-home-clock-12h": "home-clock-widget-12h-540x1080",
    "nav-home-longpress": "home-longpress-pixel",
    "sh-apps-menu": "sh-apps-menu-hold-pixel",
}
# boards that are specs or maps, not screens
NOT_SCREENS = ("-spec-", "-replacement-map", "-prototype")
STATUS_BAR = 25  # dp at the top of every 360x720 board
GAP = 6


def pairs(boards: pathlib.Path, snaps: pathlib.Path, only: str):
    names = sorted(p.name[: -len(".dc.html")] for p in boards.glob(f"{only}*.dc.html"))
    names = [n for n in names if not any(x in n for x in NOT_SCREENS)]
    # an alias may point outside the prefix (nav-openapps -> switcher-...)
    every = {p.stem for p in snaps.glob("*.png")}
    shots = {s for s in every if s.startswith(only)}
    found, missing = [], []
    for n in names:
        s = SNAPSHOT_FOR.get(n, n)
        (found if s in every else missing).append((n, s))
    used = {s for _, s in found}
    return found, [n for n, _ in missing], sorted(shots - used)


def side_by_side(
    board: Image.Image, snap: Image.Image, name: str, shot: str
) -> Image.Image:
    snap = snap.convert("RGB")
    snap = snap.resize((360, round(snap.height * 360 / snap.width)))
    board = board.convert("RGB").crop(
        (0, STATUS_BAR, 360, min(720, STATUS_BAR + snap.height))
    )
    h = max(board.height, snap.height)
    out = Image.new("RGB", (720 + GAP, h + 18), (60, 60, 60))
    out.paste(board, (0, 18))
    out.paste(snap, (360 + GAP, 18))
    d = ImageDraw.Draw(out)
    d.text((4, 3), f"canvas {name}", fill=(255, 220, 0))
    d.text((364 + GAP, 3), f"app {shot}", fill=(0, 255, 120))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description="Set each canvas board beside the app's snapshot.")
    ap.add_argument("boards", type=pathlib.Path)
    ap.add_argument(
        "--snapshots",
        type=pathlib.Path,
        default=pathlib.Path(__file__).parent.parent / "snapshots",
    )
    ap.add_argument("--out", type=pathlib.Path, default=pathlib.Path("board-check"))
    ap.add_argument(
        "--only", default="set2-", help="name prefix of the boards to check"
    )
    a = ap.parse_args()
    found, no_snap, no_board = pairs(a.boards, a.snapshots, a.only)
    a.out.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as pw:
        page = pw.chromium.launch().new_page(viewport={"width": 360, "height": 720})
        for name, shot in found:
            page.goto(
                (a.boards / f"{name}.dc.html").resolve().as_uri(), wait_until="load"
            )
            raw = a.out / f".{name}.board.png"
            page.screenshot(path=raw)
            img = side_by_side(
                Image.open(raw), Image.open(a.snapshots / f"{shot}.png"), name, shot
            )
            img.save(a.out / f"{name}.png")
            raw.unlink()
    print(f"{len(found)} pairs in {a.out}")
    if no_snap:
        print("boards without a snapshot:", " ".join(no_snap))
    if no_board:
        print("snapshots without a board:", " ".join(no_board))
    return 0


if __name__ == "__main__":
    sys.exit(main())
