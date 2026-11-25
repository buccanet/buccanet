# /// script
# requires-python = ">=3.11"
# dependencies = ["resvg-py>=0.3"]
# ///
"""Renders the master icon `buccanet-icon.png` from its vector source `buccanet-icon.svg`.

    uv run design/logo/render-icon.py

Run it after a change to the mark. `build-icons.py` reads the PNG, so the app builds need Pillow only.
"""

from pathlib import Path

import resvg_py

HERE = Path(__file__).resolve().parent
SOURCE = HERE / "buccanet-icon.svg"
MASTER = HERE / "buccanet-icon.png"
SIZE = 1024


def main() -> None:
    png = resvg_py.svg_to_bytes(svg_path=str(SOURCE), width=SIZE, height=SIZE)
    MASTER.write_bytes(bytes(png))
    print(f"rendered {MASTER.name} at {SIZE} px from {SOURCE.name}")


if __name__ == "__main__":
    main()
