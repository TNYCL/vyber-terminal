"""Regenerate the embedded file and interface icons under assets/icons.

File icons come from Seti UI (MIT) and are re-framed so that every glyph renders
at the same optical size inside a 16 px slot. Interface icons are copied from the
Lucide set that ships with gpui-kit-assets (ISC) with a slightly thinner stroke.

Usage: python scripts/build_file_icons.py <seti-ui checkout> <gpui-kit-assets dir>
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SETI = [
    "rust", "go", "typescript", "javascript", "react", "python", "json", "config",
    "yml", "lock", "shell", "powershell", "windows", "html", "css", "sass", "less",
    "svg", "image", "db", "xml", "csv", "zip", "pdf", "java", "kotlin", "c", "cpp",
    "c-sharp", "ruby", "php", "swift", "lua", "zig", "vue", "svelte", "dart", "elixir",
    "haskell", "scala", "terraform", "graphql", "wasm", "font", "audio", "video",
    "notebook", "docker", "git_ignore", "makefile", "license", "info", "npm", "yarn",
    "tsconfig", "vite", "eslint", "favicon", "prisma", "tex",
]
LUCIDE = [
    "chevron-right", "chevron-down", "chevron-up", "search", "x", "plus", "copy", "check",
    "pin", "pin-off", "eye", "maximize-2", "minimize-2", "panel-right-close", "folder",
    "folder-open", "folder-git-2", "files", "git-compare", "git-branch", "file", "file-text",
    "external-link", "arrow-up", "arrow-down", "rotate-ccw", "save", "text-search",
    "columns-2", "rows-2", "unfold-vertical", "fold-vertical", "circle-dot", "circle-check",
    "flag", "zoom-in", "zoom-out", "scan", "code", "book-open", "triangle-alert", "file-x",
    "image", "refresh-cw", "folder-search", "chevrons-up-down", "app-window", "file-diff",
    "panel-right",
]
# The Markdown mark by Dustin Curtis is dedicated to the public domain (CC0).
MARKDOWN = "M30 98V30h20l20 25 20-25h20v68H90V59L70 84 50 59v39zm125 0l-30-33h20V30h20v35h20z"

NUMBER = re.compile(r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?")
ARITY = {"M": 2, "L": 2, "H": 1, "V": 1, "C": 6, "S": 4, "Q": 4, "T": 2, "A": 7, "Z": 0}


def tokenize(d):
    # Arc flags are single digits that may be packed together ("a2 2 0 011 1").
    tokens, command, count, pos = [], None, 0, 0
    while pos < len(d):
        ch = d[pos]
        if ch.isalpha():
            tokens.append(ch)
            command, count = ch.upper(), 0
            pos += 1
        elif ch in " ,\t\n\r":
            pos += 1
        elif command == "A" and count % 7 in (3, 4):
            tokens.append(ch)
            count += 1
            pos += 1
        else:
            match = NUMBER.match(d, pos)
            tokens.append(match.group(0))
            count += 1
            pos = match.end()
    return tokens


def path_points(d):
    tokens = tokenize(d)
    x = y = sx = sy = 0.0
    command = None
    i = 0
    while i < len(tokens):
        if tokens[i].isalpha():
            command = tokens[i]
            i += 1
            if command in "Zz":
                x, y = sx, sy
                continue
        upper = command.upper()
        arity = ARITY[upper]
        args = [float(t) for t in tokens[i : i + arity]]
        i += arity
        relative = command.islower()
        if upper == "H":
            x = args[0] + (x if relative else 0)
            yield x, y
        elif upper == "V":
            y = args[0] + (y if relative else 0)
            yield x, y
        elif upper == "A":
            x = args[5] + (x if relative else 0)
            y = args[6] + (y if relative else 0)
            yield x, y
        else:
            base = (x, y) if relative else (0.0, 0.0)
            for j in range(0, arity, 2):
                yield args[j] + base[0], args[j + 1] + base[1]
            x, y = args[-2] + base[0], args[-1] + base[1]
        if upper == "M":
            sx, sy = x, y
            command = "l" if relative else "L"


def bounds(svg):
    points = []
    for d in re.findall(r'\sd="([^"]+)"', svg):
        points.extend(path_points(d))
    for attrs in re.findall(r"<(?:circle|ellipse)([^>]*)>", svg):
        get = lambda k: float(re.search(rf'\s{k}="([^"]+)"', attrs).group(1))
        cx, cy = get("cx"), get("cy")
        rx = get("r") if " r=" in attrs else get("rx")
        ry = get("r") if " r=" in attrs else get("ry")
        points += [(cx - rx, cy - ry), (cx + rx, cy + ry)]
    for attrs in re.findall(r"<rect([^>]*)>", svg):
        get = lambda k: float(re.search(rf'\s{k}="([^"]+)"', attrs).group(1))
        x, y = get("x") if " x=" in attrs else 0, get("y") if " y=" in attrs else 0
        points += [(x, y), (x + get("width"), y + get("height"))]
    for pts in re.findall(r'points="([^"]+)"', svg):
        values = [float(v) for v in NUMBER.findall(pts)]
        points += list(zip(values[0::2], values[1::2]))
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    return min(xs), min(ys), max(xs), max(ys)


def frame(svg, box):
    # Seti draws on a 32 unit grid with a 4 unit margin; keep that scale so glyphs
    # stay proportional to each other and only widen the frame when a glyph needs it.
    minx, miny, maxx, maxy = box
    if re.search(r'viewBox="0 0 32 32"', svg):
        half = max(12.0, 16 - minx, maxx - 16, 16 - miny, maxy - 16) + 0.25
        cx = cy = 16.0
    else:
        half = max(maxx - minx, maxy - miny) * 0.62
        cx, cy = (minx + maxx) / 2, (miny + maxy) / 2
    view = f"{cx - half:.2f} {cy - half:.2f} {2 * half:.2f} {2 * half:.2f}"
    svg = re.sub(r'viewBox="[^"]+"', f'viewBox="{view}"', svg, count=1)
    return re.sub(r"\s(?:id|data-name)=\"[^\"]*\"", "", svg)


def main():
    seti, kit = map(pathlib.Path, sys.argv[1:3])
    files = ROOT / "assets/icons/files"
    ui = ROOT / "assets/icons/ui"
    files.mkdir(parents=True, exist_ok=True)
    ui.mkdir(parents=True, exist_ok=True)
    for name in SETI:
        svg = (seti / "icons" / f"{name}.svg").read_text(encoding="utf-8")
        (files / f"{name}.svg").write_text(frame(svg, bounds(svg)), encoding="utf-8")
    center, half = (107.5, 64.0), 155 * 24 / 20 / 2
    (files / "markdown.svg").write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="'
        f'{center[0] - half:.2f} {center[1] - half:.2f} {2 * half:.2f} {2 * half:.2f}">'
        f'<path d="{MARKDOWN}"/></svg>',
        encoding="utf-8",
    )
    for name in LUCIDE:
        svg = (kit / "assets/icons" / f"{name}.svg").read_text(encoding="utf-8")
        svg = svg.replace('stroke-width="2"', 'stroke-width="1.75"')
        (ui / f"{name}.svg").write_text(svg, encoding="utf-8")
    print(f"{len(SETI) + 1} file icons, {len(LUCIDE)} interface icons")


if __name__ == "__main__":
    main()
