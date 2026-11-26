#!/usr/bin/env python3
"""Render the Markdown user guide in docs/ to plain HTML pages for the offline copy.

Usage: docs_to_html.py <docs dir> <output dir>

Self-contained (standard library only). It handles the small Markdown subset the guide
uses: headings, paragraphs, bullet and numbered lists, tables, fenced code, inline code,
bold, italic, links and images. Links to other .md pages become .html (README.md becomes
index.html). Images are copied to <output dir>/images. It stops with an error when a page
links to a guide page, anchor or image that doesn't exist, so a broken guide never ships.
"""
import html
import os
import re
import shutil
import sys

CSS = """
body { margin: 0 auto; max-width: 60em; padding: 1.5em 2em 3em; font: 1.05em/1.55 "Segoe UI", system-ui, sans-serif; color: #111; background: #fff; }
h1, h2, h3 { line-height: 1.25; margin: 1.4em 0 .5em; }
h1 { font-size: 1.9em; margin-top: .4em; }
h2 { font-size: 1.4em; border-bottom: 2px solid #111; padding-bottom: .15em; }
a { color: #0645ad; text-decoration: underline; }
a:visited { color: #4b2a85; }
a:focus { outline: 3px solid #111; outline-offset: 2px; }
code { font-family: Consolas, "Cascadia Mono", monospace; background: #eee; padding: 0 .25em; border-radius: 3px; }
pre { background: #f2f2f2; border: 1px solid #555; padding: .8em 1em; overflow-x: auto; }
pre code { background: none; padding: 0; }
img { max-width: 100%; height: auto; border: 1px solid #555; }
table { border-collapse: collapse; margin: 1em 0; }
th, td { border: 1px solid #555; padding: .35em .7em; text-align: left; vertical-align: top; }
th { background: #e8e8e8; }
@media (prefers-color-scheme: dark) {
  body { color: #f2f2f2; background: #111; }
  h2 { border-color: #f2f2f2; }
  a { color: #8cb4ff; } a:visited { color: #c9a8ff; } a:focus { outline-color: #ffd400; }
  code { background: #2a2a2a; } pre { background: #1c1c1c; border-color: #888; }
  th { background: #262626; } th, td, img { border-color: #888; }
}
"""


def slug(text):
    """GitHub-style heading anchor."""
    s = re.sub(r"<[^>]+>", "", text).strip().lower()
    s = re.sub(r"[^\w\- ]", "", s)
    return s.replace(" ", "-")


def html_name(md):
    base = os.path.basename(md)
    return "index.html" if base.lower() == "readme.md" else base[:-3] + ".html"


class Page:
    def __init__(self, name, text):
        self.name = name
        self.text = text
        self.anchors = set()
        self.links = []  # (target page, anchor) to check afterwards
        self.images = []


def inline(s, page):
    """Inline Markdown → HTML. Code spans first, so nothing inside them is touched."""
    out = []
    parts = re.split(r"(`[^`]+`)", s)
    for part in parts:
        if part.startswith("`") and part.endswith("`") and len(part) > 1:
            out.append("<code>" + html.escape(part[1:-1]) + "</code>")
            continue
        t = html.escape(part, quote=False)
        t = t.replace("\\*", "&#42;")

        def img(m):
            alt, src = m.group(1), m.group(2)
            page.images.append(src)
            return '<img src="%s" alt="%s">' % (html.escape(src), html.escape(alt))

        def link(m):
            label, href = m.group(1), m.group(2)
            if not re.match(r"[a-z]+:", href):
                path, _, anchor = href.partition("#")
                if path.endswith(".md"):
                    page.links.append((os.path.basename(path), anchor))
                    href = html_name(path) + ("#" + anchor if anchor else "")
                elif not path and anchor:
                    page.links.append((page.name, anchor))
            return '<a href="%s">%s</a>' % (html.escape(href), label)

        t = re.sub(r"!\[([^\]]*)\]\(([^)\s]+)\)", img, t)
        t = re.sub(r"\[([^\]]+)\]\(([^)\s]+)\)", link, t)
        t = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", t)
        t = re.sub(r"(?<![\w*])\*(?!\s)(.+?)(?<!\s)\*(?![\w*])", r"<em>\1</em>", t)
        out.append(t)
    return "".join(out)


def render(page):
    lines = page.text.splitlines()
    out, para, i = [], [], 0
    title = page.name

    def flush():
        if para:
            out.append("<p>" + inline(" ".join(para), page) + "</p>")
            para.clear()

    while i < len(lines):
        line = lines[i]
        st = line.strip()
        if st.startswith("```"):
            flush()
            i += 1
            code = []
            while i < len(lines) and not lines[i].strip().startswith("```"):
                code.append(lines[i])
                i += 1
            out.append("<pre><code>" + html.escape("\n".join(code)) + "</code></pre>")
            i += 1
            continue
        m = re.match(r"(#{1,6})\s+(.*)", st)
        if m:
            flush()
            level, text = len(m.group(1)), m.group(2).strip()
            body = inline(text, page)
            a = slug(body)
            page.anchors.add(a)
            if level == 1:
                title = re.sub(r"<[^>]+>", "", body)
            out.append('<h%d id="%s">%s</h%d>' % (level, a, body, level))
            i += 1
            continue
        if st.startswith("|") and i + 1 < len(lines) and re.match(r"^\|?\s*:?-+", lines[i + 1].strip()):
            flush()
            cells = lambda l: [c.strip() for c in l.strip().strip("|").split("|")]
            head = cells(st)
            out.append("<table><thead><tr>" + "".join("<th>%s</th>" % inline(c, page) for c in head) + "</tr></thead><tbody>")
            i += 2
            while i < len(lines) and lines[i].strip().startswith("|"):
                out.append("<tr>" + "".join("<td>%s</td>" % inline(c, page) for c in cells(lines[i])) + "</tr>")
                i += 1
            out.append("</tbody></table>")
            continue
        lm = re.match(r"(\s*)([-*]|\d+\.)\s+(.*)", line)
        if lm:
            flush()
            ordered = lm.group(2)[0].isdigit()
            tag = "ol" if ordered else "ul"
            out.append("<%s>" % tag)
            while i < len(lines):
                lm = re.match(r"(\s*)([-*]|\d+\.)\s+(.*)", lines[i])
                if not lm or lm.group(2)[0].isdigit() != ordered:
                    break
                item = [lm.group(3)]
                i += 1
                # continuation lines (indented, not a new item)
                while i < len(lines) and lines[i].startswith("  ") and lines[i].strip() and not re.match(r"\s*([-*]|\d+\.)\s+", lines[i]):
                    item.append(lines[i].strip())
                    i += 1
                out.append("<li>" + inline(" ".join(item), page) + "</li>")
                while i < len(lines) and not lines[i].strip() and i + 1 < len(lines) and re.match(r"(\s*)([-*]|\d+\.)\s+", lines[i + 1]) \
                        and re.match(r"(\s*)([-*]|\d+\.)", lines[i + 1]).group(2)[0].isdigit() == ordered:
                    i += 1
            out.append("</%s>" % tag)
            continue
        if not st:
            flush()
        else:
            para.append(st)
        i += 1
    flush()
    body = "\n".join(out)
    return (
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n"
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n"
        "<title>%s – Encode FFS</title>\n<style>%s</style>\n</head>\n<body>\n<main>\n%s\n</main>\n</body>\n</html>\n"
        % (html.escape(title), CSS, body)
    )


def main(argv):
    if len(argv) != 3:
        sys.exit(__doc__)
    src, dst = argv[1], argv[2]
    names = sorted(n for n in os.listdir(src) if n.endswith(".md"))
    pages = {}
    rendered = {}
    for n in names:
        with open(os.path.join(src, n), encoding="utf-8") as f:
            pages[n] = Page(n, f.read())
        rendered[n] = render(pages[n])
    problems = []
    for p in pages.values():
        for target, anchor in p.links:
            if target not in pages:
                problems.append("%s: link to missing page %s" % (p.name, target))
            elif anchor and anchor not in pages[target].anchors:
                problems.append("%s: link to missing heading %s#%s" % (p.name, target, anchor))
        for im in p.images:
            if not os.path.isfile(os.path.join(src, im)):
                problems.append("%s: missing image %s" % (p.name, im))
    if problems:
        sys.exit("docs_to_html: broken guide links:\n  " + "\n  ".join(problems))
    os.makedirs(dst, exist_ok=True)
    for n, text in rendered.items():
        with open(os.path.join(dst, html_name(n)), "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
    used = sorted({im for p in pages.values() for im in p.images})
    for im in used:
        os.makedirs(os.path.dirname(os.path.join(dst, im)) or dst, exist_ok=True)
        shutil.copyfile(os.path.join(src, im), os.path.join(dst, im))
    print("docs_to_html: %d pages, %d images -> %s" % (len(rendered), len(used), dst))


if __name__ == "__main__":
    main(sys.argv)
