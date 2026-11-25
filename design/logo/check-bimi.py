"""Checks `bimi.svg` against the SVG Tiny Portable/Secure profile that BIMI requires for the logo of a mail domain.

    python3 design/logo/check-bimi.py [file]

The rules come from the BIMI Group's SVG Tiny PS draft: `version="1.2"` and `baseProfile="tiny-ps"` on the root,
a `<title>`, a square `viewBox`, no `x` or `y` on the root, no script, animation, raster image or link, no reference
outside the file, and a file of 32 KB at most. The script exits with status 1 and lists every broken rule.
"""

import re
import sys
from pathlib import Path
from xml.etree import ElementTree

SVG = "http://www.w3.org/2000/svg"
DEFAULT = Path(__file__).resolve().parent / "bimi.svg"
MAX_BYTES = 32 * 1024
# The drawing elements of SVG Tiny 1.2 that the PS profile keeps. Script, animation, image, link, font and
# foreignObject elements are left out.
ALLOWED = {
    "svg",
    "title",
    "desc",
    "g",
    "defs",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "solidColor",
    "linearGradient",
    "radialGradient",
    "stop",
}
EXTERNAL_URL = re.compile(r"url\(\s*['\"]?(?!#)", re.IGNORECASE)


def local(name: str) -> str:
    return name.rsplit("}", 1)[-1]


def root_errors(root: ElementTree.Element) -> list[str]:
    errors = []
    if root.tag != f"{{{SVG}}}svg":
        errors.append(f"the root is {root.tag}, an svg element in the {SVG} namespace is required")
    if root.get("version") != "1.2":
        errors.append('the root needs version="1.2"')
    if root.get("baseProfile") != "tiny-ps":
        errors.append('the root needs baseProfile="tiny-ps"')
    errors += [f"the root carries {name}=, which the profile forbids" for name in ("x", "y") if name in root.attrib]
    box = (root.get("viewBox") or "").replace(",", " ").split()
    if len(box) != 4:
        errors.append("the root needs a viewBox of four numbers")
    elif float(box[2]) != float(box[3]):
        errors.append(f"the viewBox is {box[2]} by {box[3]}, a square is required")
    titles = [child for child in root if child.tag == f"{{{SVG}}}title"]
    if not titles or not (titles[0].text or "").strip():
        errors.append("the root needs a <title> child with the brand name")
    return errors


def element_errors(root: ElementTree.Element) -> list[str]:
    errors = []
    for element in root.iter():
        name = local(element.tag)
        if not element.tag.startswith(f"{{{SVG}}}") or name not in ALLOWED:
            errors.append(f"<{name}> is outside the elements SVG Tiny PS allows")
        for attribute, value in element.attrib.items():
            key = local(attribute)
            if key.lower().startswith("on"):
                errors.append(f"<{name}> carries the event attribute {key}")
            if key == "href" and not value.startswith("#"):
                errors.append(f"<{name}> references {value} outside the file")
            if EXTERNAL_URL.search(value):
                errors.append(f"<{name}> {key} references a url outside the file")
    return errors


def check(path: Path) -> list[str]:
    size = path.stat().st_size
    errors = [f"the file is {size} bytes, BIMI accepts {MAX_BYTES} at most"] if size > MAX_BYTES else []
    root = ElementTree.parse(path).getroot()
    return errors + root_errors(root) + element_errors(root)


def main(arguments: list[str]) -> None:
    path = Path(arguments[0]) if arguments else DEFAULT
    errors = check(path)
    for error in errors:
        print(f"{path.name}: {error}")
    if errors:
        sys.exit(1)
    print(f"{path.name}: valid SVG Tiny PS for BIMI, {path.stat().st_size} bytes")


if __name__ == "__main__":
    main(sys.argv[1:])
