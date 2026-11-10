#!/usr/bin/env python3
"""Builds the landing page into one folder, the way GitHub Pages serves it.

    python3 site/build.py <out dir>

site/template.html is rendered once per language from site/locales/<lang>.toml: English at /, the others at
/<lang>/. `{{key}}` puts a locale value in as HTML, `{{key:attr}}` escaped for an attribute. The static files and
docs/screenshots/*.png (as img/) are copied next to the pages. Links inside the pages are relative (`{{root}}`), so
the site works both on a custom domain and at <user>.github.io/rgb-tray/.
Every language must have exactly the keys English has, and every placeholder must be filled; anything else stops
the build. Standard library only (Python 3.11+ for tomllib).
"""

import html
import re
import shutil
import sys
import tomllib
from pathlib import Path

SITE = Path(__file__).resolve().parent
ROOT = SITE.parent
BASE = "https://rgb-tray.gronck.com/"
REPO = "https://github.com/Gronck2/rgb-tray"
# English is the default and is served at /, the others at /<code in lower case>/ (GitHub Pages paths are
# case-sensitive, and 404.html sends /RU/ or /zh-CN/ there).
LANGS = ["en", "ru", "zh-CN", "es", "de", "fr", "pt-BR", "ja"]
NAMES = {
    "en": "English",
    "ru": "Русский",
    "zh-CN": "简体中文",
    "es": "Español",
    "de": "Deutsch",
    "fr": "Français",
    "pt-BR": "Português (Brasil)",
    "ja": "日本語",
}
# Short labels of the language switch.
SHORT = {"en": "EN", "ru": "RU", "zh-CN": "中文", "es": "ES", "de": "DE", "fr": "FR", "pt-BR": "PT", "ja": "日本語"}
# Every language has a README; CONTRIBUTING and the profile docs exist in English and Russian only.
DOCS_IN = {"en", "ru"}
# Screenshots of the gallery, in order (docs/screenshots, rendered with --snapshot).
SHOTS = ["panel-dark", "panel-light", "editor-dark"]
# The app's interface is in English and Russian: docs/screenshots/<view>.png show the Russian one (the README uses
# them), <view>-en.png the English one (rendered by .github/workflows/screenshots.yml), for every other language.
UI_LANGS = {"ru": ""}


def shot(lang, view):
    return f"{view}{UI_LANGS.get(lang, '-en')}.png"


def png_size(name):
    """Width and height of docs/screenshots/<name>, from the PNG header."""
    path = ROOT / "docs" / "screenshots" / name
    if not path.exists():
        fail(f"docs/screenshots/{name} is missing")
    head = path.read_bytes()[:24]
    if head[:8] != b"\x89PNG\r\n\x1a\n":
        fail(f"docs/screenshots/{name} is not a PNG")
    return int.from_bytes(head[16:20], "big"), int.from_bytes(head[20:24], "big")
STATIC = ["style.css", "site.js", "404.html", "favicon.svg", "fonts"]

CURRENT = ' aria-current="page"'
PLACEHOLDER = re.compile(r"\{\{([a-z0-9_]+)(:attr)?\}\}")


def fail(msg):
    sys.exit(f"build.py: {msg}")


def page_path(lang):
    return "" if lang == "en" else f"{lang.lower()}/"


def suffix(lang):
    return "" if lang == "en" else f".{lang}"


def load(lang):
    with open(SITE / "locales" / f"{lang}.toml", "rb") as f:
        return tomllib.load(f)


def check_same_shape(lang, loc, ref):
    if set(loc) != set(ref):
        missing, extra = sorted(set(ref) - set(loc)), sorted(set(loc) - set(ref))
        fail(f"{lang}.toml: missing {missing}, extra {extra}")
    for key, val in ref.items():
        if isinstance(val, list):
            if len(loc[key]) != len(val):
                fail(f"{lang}.toml: [[{key}]] has {len(loc[key])} entries, en has {len(val)}")
            for i, (a, b) in enumerate(zip(loc[key], val)):
                if set(a) != set(b):
                    fail(f"{lang}.toml: [[{key}]] #{i + 1} has keys {sorted(a)}, en has {sorted(b)}")
                for k in ("hue", "view", "cmd"):
                    if k in b and a[k] != b[k]:
                        fail(f"{lang}.toml: [[{key}]] #{i + 1} {k} = {a[k]!r}, en has {b[k]!r}")


def blocks(lang, loc):
    """The repeated parts and the values that depend on the language, not on the text."""
    a = lambda s: html.escape(s, quote=True)
    root = "" if lang == "en" else "../"
    doc = suffix(lang) if lang in DOCS_IN else ""
    out = {}
    out["lang"] = lang
    out["root"] = root
    out["home_path"] = page_path(lang) or "./"
    out["page_url"] = BASE + page_path(lang)
    out["og_image"] = f"{BASE}img/{shot(lang, 'panel-dark')}"
    out["hero_dark"] = f"{root}img/{shot(lang, 'panel-dark')}"
    out["hero_light"] = f"{root}img/{shot(lang, 'panel-light')}"
    out["hero_w"], out["hero_h"] = (str(n) for n in png_size(shot(lang, "panel-dark")))
    out["readme_url"] = f"{REPO}/blob/main/README{suffix(lang)}.md"
    out["contributing_url"] = f"{REPO}/blob/main/CONTRIBUTING{doc}.md"
    out["profiles_doc_url"] = f"{REPO}/blob/main/docs/profiles{doc}.md"
    out["capture_doc_url"] = f"{REPO}/blob/main/docs/usb-capture{doc}.md"
    out["alternates"] = "\n".join(
        [f'<link rel="alternate" hreflang="{l}" href="{BASE}{page_path(l)}">' for l in LANGS]
        + [f'<link rel="alternate" hreflang="x-default" href="{BASE}">']
    )
    out["lang_name"] = NAMES[lang]
    out["lang_short"] = SHORT[lang]
    out["lang_links"] = "".join(
        f'<li><a href="{root}{page_path(l)}" hreflang="{l}" lang="{l}"{CURRENT if l == lang else ""}>{NAMES[l]}</a></li>'
        for l in LANGS
    )
    out["why_list"] = "\n".join(f'<li><h3>{w["title"]}</h3><p>{w["text"]}</p></li>' for w in loc["why"])
    out["command_list"] = "\n".join(f'<tr><td><code>{html.escape(c["cmd"])}</code></td><td>{c["text"]}</td></tr>' for c in loc["commands"])
    out["family_list"] = "\n".join(
        f'<article class="family" style="--hue: {f["hue"]}"><span class="led" aria-hidden="true"></span>'
        f'<h3>{f["title"]}</h3><p class="models">{f["models"]}</p><p>{f["text"]}</p></article>'
        for f in loc["families"]
    )
    out["feature_list"] = "\n".join(f'<li><h3>{f["title"]}</h3><p>{f["text"]}</p></li>' for f in loc["features"])
    out["limit_list"] = "\n".join(f'<li><strong>{x["head"]}</strong> {x["text"]}</li>' for x in loc["limits"])
    if [s["view"] for s in loc["shots"]] != list(SHOTS):
        fail(f"{lang}.toml: [[shots]] views must be {list(SHOTS)} in this order")
    out["shot_list"] = "\n".join(
        f'<figure class="{s["view"].split("-")[0]}"><a class="shot zoom" href="{root}img/{shot(lang, s["view"])}">'
        f'<img src="{root}img/{shot(lang, s["view"])}" width="{png_size(shot(lang, s["view"]))[0]}" height="{png_size(shot(lang, s["view"]))[1]}" '
        f'alt="{a(s["alt"])}" loading="lazy"></a><figcaption>{s["caption"]}</figcaption></figure>'
        for s in loc["shots"]
    )
    out["claude_steps"] = "\n".join(f'<li><h4>{s["title"]}</h4><p>{s["text"]}</p></li>' for s in loc["claude_steps"])
    out["route_list"] = "\n".join(f'<li><span class="route-n">{i}</span><div><h4>{r["title"]}</h4><p>{r["text"]}</p></div></li>' for i, r in enumerate(loc["routes"], 1))
    out["share_steps"] = "\n".join(f'<li>{s["text"]}</li>' for s in loc["share_steps"])
    out["install_steps"] = "\n".join(f'<li>{s["text"]}</li>' for s in loc["install_steps"])
    return out


def render(template, lang, loc):
    values = {k: v for k, v in loc.items() if isinstance(v, str)}
    for k, v in blocks(lang, loc).items():
        if k in values:
            fail(f"{lang}.toml: '{k}' is computed by build.py, remove it from the locale")
        values[k] = v
    used = set()

    def sub(m):
        key, attr = m.group(1), m.group(2)
        if key not in values:
            fail(f"template.html: {{{{{key}}}}} has no value in {lang}.toml")
        used.add(key)
        return html.escape(values[key], quote=True) if attr else values[key]

    page = PLACEHOLDER.sub(sub, template)
    # A locale value may use {{root}} (and the doc links) itself, for links inside the page.
    page = PLACEHOLDER.sub(sub, page)
    unused = sorted(k for k, v in loc.items() if isinstance(v, str) and k not in used)
    if unused:
        fail(f"{lang}.toml: keys not used by template.html: {unused}")
    if "{{" in page:
        fail(f"template.html: a malformed placeholder is left in the {lang} page")
    return page


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    out = Path(sys.argv[1])
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    template = (SITE / "template.html").read_text(encoding="utf-8")
    ref = load("en")
    for lang in LANGS:
        loc = load(lang)
        check_same_shape(lang, loc, ref)
        dest = out / page_path(lang) / "index.html"
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(render(template, lang, loc), encoding="utf-8")

    for name in STATIC:
        src = SITE / name
        if src.is_dir():
            shutil.copytree(src, out / name)
        else:
            shutil.copy2(src, out / name)
    notfound = (SITE / "404.html").read_text(encoding="utf-8")
    for lang in LANGS[1:]:
        if f'data-path="{page_path(lang)}"' not in notfound:
            fail(f"404.html has no link to /{page_path(lang)}")
    img = out / "img"
    img.mkdir()
    for name in sorted({shot(lang, view) for lang in LANGS for view in SHOTS}):
        shutil.copy2(ROOT / "docs" / "screenshots" / name, img / name)
    print(f"build.py: {len(LANGS)} pages in {out}")


if __name__ == "__main__":
    main()
