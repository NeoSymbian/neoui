# SPDX-License-Identifier: Apache-2.0
"""NeoSans: Inter with Noto Sans Armenian's letters, at 400, 500, 600 and 700 (docs neoui-kit.md, "Fonts").

Slint draws a Text in one font with no fallback, and Inter has no Armenian: the design (the canvas, Inter with Noto
Sans Armenian after it) needs one family with both. Static cuts of both variable fonts are subset to the scripts the
UI draws (Latin, Cyrillic, Armenian, punctuation and the symbols it uses), Noto is scaled to Inter's 2048 units, and
each pair is merged into ui/fonts/NeoSans-{Regular,Medium,SemiBold,Bold}.ttf.

    uv run --with fonttools python tools/make_fonts.py <NotoSansArmenian[wdth,wght].ttf>

Both fonts are OFL-1.1: ui/fonts/NeoSans.ttf.license carries both notices.
"""

import sys
from pathlib import Path

from fontTools.merge import Merger
from fontTools.subset import Options, Subsetter
from fontTools.ttLib import TTFont
from fontTools.ttLib.scaleUpem import scale_upem
from fontTools.varLib.instancer import instantiateVariableFont

FONTS = Path(__file__).resolve().parent.parent / "ui" / "fonts"
WEIGHTS = {400: "Regular", 500: "Medium", 600: "SemiBold", 700: "Bold"}

# Basic Latin to Latin Extended-B, Greek and Cyrillic, general punctuation, currency, letterlike, arrows, the maths and
# shapes the UI draws (✓ ✕ ★ ⚠ ● …), and the Armenian block with its ligatures.
LATIN = [*range(0x20, 0x250), *range(0x370, 0x400), *range(0x400, 0x530), *range(0x1E00, 0x1F00)]
SYMBOLS = [*range(0x2000, 0x2070), *range(0x20A0, 0x20C1), *range(0x2100, 0x2150), *range(0x2190, 0x2200),
           *range(0x2200, 0x2300), *range(0x25A0, 0x2600), *range(0x2600, 0x2700), *range(0x2700, 0x27C0)]
ARMENIAN = [*range(0x530, 0x590), *range(0xFB13, 0xFB18)]


def cut(path: Path, axes: dict, unicodes: list) -> TTFont:
    font = instantiateVariableFont(TTFont(path), axes)
    options = Options()
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.notdef_outline = True
    subsetter = Subsetter(options)
    subsetter.populate(unicodes=unicodes)
    subsetter.subset(font)
    return font


def main(noto: Path) -> None:
    for weight, style in WEIGHTS.items():
        inter = cut(FONTS / "Inter-VariableFont.ttf", {"wght": weight}, LATIN + SYMBOLS)
        armenian = cut(noto, {"wght": weight, "wdth": 100}, ARMENIAN)
        scale_upem(armenian, inter["head"].unitsPerEm)
        tmp = [FONTS / f".inter-{weight}.ttf", FONTS / f".armenian-{weight}.ttf"]
        inter.save(tmp[0])
        armenian.save(tmp[1])
        merged = Merger().merge([str(p) for p in tmp])
        for p in tmp:
            p.unlink()
        name = merged["name"]
        for record in list(name.names):
            if record.nameID in (1, 2, 3, 4, 6, 16, 17):
                name.removeNames(nameID=record.nameID)
        full = f"NeoSans {style}"
        for nid, text in {1: "NeoSans", 2: style, 3: f"NeoSans-{style}", 4: full, 6: f"NeoSans-{style}"}.items():
            name.setName(text, nid, 3, 1, 0x409)
        merged["OS/2"].usWeightClass = weight
        out = FONTS / f"NeoSans-{style}.ttf"
        merged.save(out)
        print(f"{out.name}: {out.stat().st_size} bytes, {len(merged.getGlyphOrder())} glyphs")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
