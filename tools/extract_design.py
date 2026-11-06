# SPDX-License-Identifier: Apache-2.0
"""extract_design -- measure the Symbian^3 360x640 touch layout and write the facts neoui's theme is built from.

    python3 tools/extract_design.py --ref ../private [--out design/measured.json]

Reads the EPL-1.0 layout data from the 2010 open release (copied to <ref>/symbian-epl, see its SOURCES.md):
the aknlayout2 component tree for qHD touch (portrait and landscape) and the scalable font table. Each wanted
component's rectangle is resolved through its parent chain (variety 0, option set 0) from the 360x640
application window. Only numbers come out -- sizes in px at 360x640, which neoui treats as dp.
"""

import argparse
import json
import os
import re
import sys
import xml.etree.ElementTree as ET

# token -> layout component (portrait unless noted). "h"/"w" etc. pick the rectangle field.
WANT = {
    # the thin (25 px) status pane: variety 1 -- the Belle-style frame
    "status_bar": ("area_top_pane", "1"),
    "top_area_with_title": ("area_top_pane", "0"),
    "title_pane": "title_pane",
    "bottom_area": "area_bottom_pane",
    "toolbar": "control_pane",
    "list_single": "list_single_pane",
    "list_single_graphic": "list_single_graphic_pane",
    "list_double": "list_double_pane",
    "list_double_graphic": "list_double_graphic_pane",
    "list_highlight": "list_highlight_pane_cp1",
    "app_grid": "grid_app_pane",
    "app_cell": "cell_app_pane",
    "touch_grid_3": "grid_touch_3_pane",
    "touch_cell_3": "cell_touch_3_pane",
    "scroll_bar": "scroll_pane",
    "toolbar_cell": "cell_toolbar2_fixed_pane",
    "indicators": "uni_indicator_pane",
    "clock": "clock_nsta_pane",
}
LANDSCAPE = {"list_single", "list_single_graphic", "list_double", "list_double_graphic", "list_highlight", "app_cell"}
FONT_CATEGORY = {
    "EAknFontCategorySecondary": "secondary",
    "EAknFontCategoryPrimarySmall": "primary_small",
    "EAknFontCategoryPrimary": "primary",
    "EAknFontCategoryTitle": "title",
    "EAknFontCategoryDigital": "digital",
}
NUM = re.compile(r"^-?\d+(\.\d+)?$")


def load_components(path: str) -> dict:
    root = ET.parse(path).getroot()
    comps = {}
    for c in root.iter("Component"):
        parents = {}
        for opt in c.iterfind("ParentInfo/option"):
            p = opt.find("parent")
            if p is not None:
                parents.setdefault(opt.get("variety"), (p.get("id"), p.get("variety", "0")))
        params: dict[str, dict[str, str]] = {}
        for prm in c.iterfind("LayoutInfo/Param"):
            for v in prm.iterfind("Value"):
                var = v.find("varietyIndex")
                calc = v.find("calc")
                key = var.get("value") if var is not None else "0"
                params.setdefault(key, {}).setdefault(prm.get("name"), calc.get("value") if calc is not None else "")
        comps[c.get("id")] = {"name": c.get("name"), "parents": parents, "params": params, "type": c.get("type")}
    return {"root": root.attrib, "by_id": comps, "by_name": {v["name"]: k for k, v in comps.items()}}


def _n(v):
    return float(v) if v is not None and NUM.match(v.strip()) else None


def rect(layout: dict, cid: str, variety: str = "0", seen=()) -> tuple | None:
    c = layout["by_id"].get(cid)
    if c is None or cid in seen:
        return None
    if c["name"] == "application_window" or not c["parents"]:
        return (0.0, 0.0, float(layout["root"]["resolutionWidth"]), float(layout["root"]["resolutionHeight"]))
    parent = c["parents"].get(variety) or c["parents"].get("0") or next(iter(c["parents"].values()))
    pr = rect(layout, parent[0], parent[1], (*seen, cid))
    if pr is None:
        return None
    px, py, pw, ph = pr
    p = c["params"].get(variety) or c["params"].get("0") or {}
    L, R, T, B = (_n(p.get(k)) for k in ("Left", "Right", "Top", "Bottom"))
    W, H = _n(p.get("Width")), _n(p.get("Height"))
    w = W if W is not None else pw - (L or 0) - (R or 0)
    h = H if H is not None else ph - (T or 0) - (B or 0)
    x = px + L if L is not None else (px + pw - R - w if R is not None else px)
    y = py + T if T is not None else (py + ph - B - h if B is not None else py)
    return (x, y, w, h)


def measure(layout: dict) -> dict:
    out = {}
    for token, want in WANT.items():
        name, variety = want if isinstance(want, tuple) else (want, "0")
        cid = layout["by_name"].get(name)
        r = rect(layout, cid, variety) if cid else None
        if r:
            out[token] = {"x": r[0], "y": r[1], "w": r[2], "h": r[3], "component": f"{name}/{variety}"}
    return out


def fonts(cpp: str) -> dict:
    """the scalable text-pane heights per category (the + EScalableFontIdOffset rows), largest per category"""
    out: dict[str, int] = {}
    for m in re.finditer(
        r"\{\s*\w+\+EScalableFontIdOffset,\s*(\w+),\s*(\d+),\s*(\d+)", cpp
    ):
        cat = FONT_CATEGORY.get(m.group(1))
        if cat:
            key = cat + ("_bold" if m.group(3) == "1" and cat in ("secondary",) else "")
            out[key] = max(out.get(key, 0), int(m.group(2)))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n")[0])
    ap.add_argument(
        "--ref", required=True, help="the private/ folder (has symbian-epl/)"
    )
    ap.add_argument(
        "--out",
        default=os.path.join(
            os.path.dirname(__file__), "..", "design", "measured.json"
        ),
    )
    a = ap.parse_args()
    epl = os.path.join(a.ref, "symbian-epl")
    res = {
        "_source": "Symbian^3 open release (EPL-1.0) aknlayout2 qHD touch + Cdlfontinst_qhd_touch.cpp; "
        "px at 360x640, 229 ppi; variety 0. Facts only.",
        "portrait": {},
        "landscape": {},
    }
    for orient, d in (
        ("portrait", "php_av_qhd_prt_tch"),
        ("landscape", "phl_av_qhd_lsc_tch"),
    ):
        lay = load_components(
            os.path.join(epl, "layouts", d, "display_eur_compData.xml")
        )
        res[orient] = measure(lay)
        if orient == "landscape":  # its frame panes need varieties we do not resolve; keep the rows and cells
            res[orient] = {k: v for k, v in res[orient].items() if k in LANDSCAPE}
        res[orient]["_screen"] = [
            int(lay["root"]["resolutionWidth"]),
            int(lay["root"]["resolutionHeight"]),
        ]
    with open(
        os.path.join(epl, "fonts", "Cdlfontinst_qhd_touch.cpp"), errors="replace"
    ) as f:
        res["text_pane_heights"] = fonts(f.read())
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    with open(a.out, "w") as f:
        json.dump(res, f, indent=1, sort_keys=True)
    for o in ("portrait", "landscape"):
        print(
            o,
            {
                k: (round(v["w"]), round(v["h"]))
                for k, v in res[o].items()
                if not k.startswith("_")
            },
        )
    print("fonts", res["text_pane_heights"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
