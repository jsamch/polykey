#!/usr/bin/env python3
"""Build docs/recovery_page.html, the printable recovery page.

The text comes only from docs/RECOVERY_CHECKLIST.md (the one source, also embedded in the GUI).
The screenshots come from docs/img/. The output is one self-contained HTML file: images are
inlined as base64 and the print CSS fits A4 and Letter on one page.

Usage:
    python3 tools/make_recovery_page.py            write docs/recovery_page.html
    python3 tools/make_recovery_page.py --check    exit 1 if the file is not up to date
"""
import base64
import html
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CHECKLIST = os.path.join(ROOT, "docs", "RECOVERY_CHECKLIST.md")
IMG_DIR = os.path.join(ROOT, "docs", "img")
OUT = os.path.join(ROOT, "docs", "recovery_page.html")

# (file name, caption). Images that do not exist are skipped.
IMAGES = [
    ("home.png", "Home: press Recover"),
    ("recover.png", "Recover: plates added, set card shows Ready"),
    ("passphrase.png", "The passphrase is shown once"),
    ("check.png", "Check: report without the passphrase"),
]

CSS = """
:root { color-scheme: light; }
@page { margin: 9mm 11mm; }
* { box-sizing: border-box; }
body { font-family: Arial, Helvetica, sans-serif; font-size: 9pt; line-height: 1.28;
       color: #000; background: #fff; margin: 0 auto; max-width: 190mm; padding: 6mm; }
h1 { font-size: 15pt; margin: 0 0 2mm; }
h2 { font-size: 10.5pt; margin: 3mm 0 1mm; border-bottom: 1px solid #000; }
p { margin: 0 0 1.5mm; }
ol, ul { margin: 0; padding-left: 6mm; }
li { margin: 0 0 0.8mm; }
.shots { display: flex; gap: 2mm; margin-top: 3mm; }
.shots figure { margin: 0; flex: 1 1 0; min-width: 0; }
.shots img { width: 100%; height: auto; border: 1px solid #000; display: block; }
.shots figcaption { font-size: 7pt; margin-top: 0.5mm; }
@media print {
  body { padding: 0; max-width: none; }
  h2, li, figure { break-inside: avoid; }
}
"""


def inline(text):
    return html.escape(text, quote=False)


def render_markdown(md):
    """Render the small subset used by the checklist: #, ##, paragraphs, numbered and bullet lists."""
    out = []
    mode = None  # None, "ol" or "ul"

    def close():
        nonlocal mode
        if mode:
            out.append("</%s>" % mode)
            mode = None

    for line in md.splitlines():
        if not line.strip():
            close()
            continue
        m = re.match(r"^(#{1,2}) (.*)$", line)
        if m:
            close()
            tag = "h%d" % len(m.group(1))
            out.append("<%s>%s</%s>" % (tag, inline(m.group(2)), tag))
            continue
        m = re.match(r"^(\d+)\. (.*)$", line)
        if m:
            if mode != "ol":
                close()
                out.append('<ol start="%s">' % m.group(1))
                mode = "ol"
            out.append("<li>%s</li>" % inline(m.group(2)))
            continue
        m = re.match(r"^- (.*)$", line)
        if m:
            if mode != "ul":
                close()
                out.append("<ul>")
                mode = "ul"
            out.append("<li>%s</li>" % inline(m.group(1)))
            continue
        close()
        out.append("<p>%s</p>" % inline(line))
    close()
    return "\n".join(out)


def figures():
    figs = []
    for name, caption in IMAGES:
        path = os.path.join(IMG_DIR, name)
        if not os.path.exists(path):
            continue
        with open(path, "rb") as f:
            data = base64.b64encode(f.read()).decode("ascii")
        figs.append(
            '<figure><img alt="%s" src="data:image/png;base64,%s">'
            "<figcaption>%s</figcaption></figure>" % (inline(caption), data, inline(caption))
        )
    if not figs:
        return ""
    return '<div class="shots">\n%s\n</div>' % "\n".join(figs)


def build():
    with open(CHECKLIST, encoding="utf-8") as f:
        md = f.read()
    title = re.match(r"# (.*)", md).group(1)
    return (
        '<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n'
        '<meta name="viewport" content="width=device-width, initial-scale=1">\n'
        "<title>%s</title>\n<style>%s</style>\n</head>\n<body>\n%s\n%s\n</body>\n</html>\n"
        % (inline(title), CSS, render_markdown(md), figures())
    )


def main():
    page = build()
    if "--check" in sys.argv[1:]:
        try:
            with open(OUT, encoding="utf-8", newline="") as f:
                current = f.read()
        except OSError:
            current = None
        if current != page:
            print("docs/recovery_page.html is out of date; run tools/make_recovery_page.py")
            return 1
        print("docs/recovery_page.html is up to date")
        return 0
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(page)
    print("wrote", os.path.relpath(OUT, ROOT))
    return 0


if __name__ == "__main__":
    sys.exit(main())
