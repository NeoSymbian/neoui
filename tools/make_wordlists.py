# SPDX-License-Identifier: Apache-2.0
"""make_wordlists -- the system keyboard's word lists (suggestions, `keyboard.suggest`) into crates/neoui-render/data/.

    python3 tools/make_wordlists.py [--words 3000] [--pages 1200] [--cache DIR]

Counts the words of public-domain texts and keeps the most frequent ones per language:
- en: Project Gutenberg books (public domain in the US; the Gutenberg header and licence footer are cut off);
- ru, hy: random pages of the main namespace of ru.wikisource.org and hy.wikisource.org (public-domain works).
Only counts leave this script, no text: a list of common words with their rank is a fact, not the texts' expression,
so the lists are ours (Apache-2.0, words-*.txt.license). Pre-1918 Russian spellings (ѣ, і, ѳ, ѵ, a final ъ) and
classical Armenian ones (ւ outside ու and և) are dropped: the keyboards type the modern spellings.

Each list is `word\\trank` per line (rank 1 = the most frequent), sorted by code point, so neoui-render's `suggest`
binary-searches a prefix in the ROM's copy without building anything on the heap.
"""

import argparse
import collections
import json
import pathlib
import re
import sys
import time
import urllib.parse
import urllib.request

OUT = (
    pathlib.Path(__file__).resolve().parent.parent / "crates" / "neoui-render" / "data"
)
UA = {"User-Agent": "wordlist-builder/1.0 (word counts of public-domain texts)"}
GUTENBERG = "https://gutenberg.pglaf.org"
# Pride and Prejudice, Sherlock Holmes, Tom Sawyer, Alice, Huckleberry Finn, Great Expectations, Frankenstein,
# A Tale of Two Cities, Dracula, The Time Machine, Treasure Island, Little Women, The Call of the Wild, Emma
EN_BOOKS = [1342, 1661, 74, 11, 76, 1400, 84, 98, 345, 35, 120, 514, 215, 158]
WORD = {
    "en": re.compile(r"[a-z]+(?:'[a-z]+)?"),
    "ru": re.compile(r"[а-яё]+"),
    "hy": re.compile(r"[ա-և]+"),
}


def get(url: str) -> bytes:
    for attempt in range(6):
        try:
            with urllib.request.urlopen(
                urllib.request.Request(url, headers=UA), timeout=60
            ) as r:
                return r.read()
        except Exception as e:
            if attempt == 5:
                raise
            print(f"  retry {url}: {e}", file=sys.stderr)
            # Wikimedia answers a burst with 429: back off for longer
            time.sleep((30 if "429" in str(e) else 3) * (attempt + 1))
    raise AssertionError


def gutenberg(n: int) -> str:
    d = "/".join(str(n)[:-1]) or "0"
    for name in (f"{n}-0.txt", f"{n}.txt"):
        try:
            t = get(f"{GUTENBERG}/{d}/{n}/{name}").decode("utf-8", "replace")
        except Exception as e:  # noqa: BLE001 - try the other name
            print(f"  Gutenberg {n} {name}: {e}", file=sys.stderr)
            continue
        a, b = t.find("*** START"), t.find("*** END")
        return t[t.find("\n", a) : b] if a >= 0 and b > a else t
    print(f"  no text for Gutenberg {n}", file=sys.stderr)
    return ""


def wikisource(lang: str, pages: int) -> str:
    out, got = [], 0
    while got < pages:
        q = urllib.parse.urlencode(
            {
                "action": "query",
                "format": "json",
                "generator": "random",
                "grnnamespace": 0,
                "grnlimit": 50,
                "prop": "revisions",
                "rvprop": "content",
                "rvslots": "main",
            }
        )
        data = json.loads(get(f"https://{lang}.wikisource.org/w/api.php?{q}"))
        for p in data.get("query", {}).get("pages", {}).values():
            for r in p.get("revisions", []):
                out.append(strip_wiki(r["slots"]["main"].get("*", "")))
                got += 1
        print(f"  {lang}: {got} pages", file=sys.stderr)
        time.sleep(2)
    return "\n".join(out)


def strip_wiki(t: str) -> str:
    for _ in range(3):
        t = re.sub(r"\{\{[^{}]*\}\}", " ", t)
    t = re.sub(r"\[\[(?:[^|\]]*\|)?([^\]]*)\]\]", r"\1", t)
    t = re.sub(r"<[^>]+>", " ", t)
    return t


def keep(lang: str, w: str) -> bool:
    if len(w) < 2:
        return False
    if lang == "ru":
        return not re.search(r"[ѣіѳѵ]", w) and not w.endswith("ъ")
    if lang == "hy":
        return "ւ" not in w.replace("ու", "")
    return True


def build(lang: str, text: str, n: int) -> list[str]:
    # category, file and interwiki links (Категория:Name, a link's text by now): not prose
    text = re.sub(r"\w+:\w+", " ", text)
    counts = collections.Counter(
        w for w in WORD[lang].findall(text.lower()) if keep(lang, w)
    )
    # names: words written with a capital most of the time (Fitzwilliam, Москва) are left out
    lower = collections.Counter(WORD[lang].findall(text))
    counts = collections.Counter({w: c for w, c in counts.items() if lower[w] * 2 >= c})
    top = [w for w, c in counts.most_common(n) if c > 1]
    rank = {w: i + 1 for i, w in enumerate(top)}
    return [f"{w}\t{rank[w]}" for w in sorted(top)]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--words", type=int, default=3000)
    ap.add_argument(
        "--pages", type=int, default=1200, help="Wikisource pages per language"
    )
    ap.add_argument(
        "--cache", type=pathlib.Path, help="keep the fetched texts here and reuse them"
    )
    a = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    fetch = {
        "en": lambda: "\n".join(gutenberg(b) for b in EN_BOOKS),
        "ru": lambda: wikisource("ru", a.pages),
        "hy": lambda: wikisource("hy", a.pages),
    }
    for lang, f in fetch.items():
        cached = a.cache / f"{lang}.txt" if a.cache else None
        if cached and cached.exists():
            text = cached.read_text(encoding="utf-8")
        else:
            text = f()
            if cached:
                cached.parent.mkdir(parents=True, exist_ok=True)
                cached.write_text(text, encoding="utf-8")
        lines = build(lang, text, a.words)
        (OUT / f"words-{lang}.txt").write_text(
            "\n".join(lines) + "\n", encoding="utf-8"
        )
        print(f"{lang}: {len(lines)} words from {len(text)} characters")
    return 0


if __name__ == "__main__":
    sys.exit(main())
