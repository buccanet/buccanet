# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow>=11"]
# ///
"""Builds an app's icons from `buccanet-icon.png`, inside that app's repository.

    uv run design/logo/build-icons.py <apple|android|web|linux> <app repository>

Each app runs it from its build script through its `vendor/buccanet` submodule. The outputs are
generated files and are not versioned.
"""

import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

MASTER = Path(__file__).resolve().parent / "buccanet-icon.png"

# macOS icon grid: the body is 824 of 1024 points, with this corner radius.
MAC_BODY = 824 / 1024
MAC_RADIUS = 185 / 824
# Android adaptive icons: 108 dp layers, of which a 72 dp square can show through the mask.
ADAPTIVE_VISIBLE = 72 / 108
ANDROID_DENSITIES = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}


def master() -> Image.Image:
    return Image.open(MASTER).convert("RGB")


def scaled(art: Image.Image, size: int) -> Image.Image:
    return art.resize((size, size), Image.Resampling.LANCZOS)


def rounded(art: Image.Image, size: int, radius: float) -> Image.Image:
    """The art clipped to a rounded square filling `size`, radius as a share of the side."""
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size - 1, size - 1), radius=round(size * radius), fill=255)
    tile = scaled(art, size).convert("RGBA")
    tile.putalpha(mask)
    return tile


def circle(art: Image.Image, size: int) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).ellipse((0, 0, size - 1, size - 1), fill=255)
    tile = scaled(art, size).convert("RGBA")
    tile.putalpha(mask)
    return tile


def mac_icon(art: Image.Image, size: int) -> Image.Image:
    """A macOS icon: the rounded body on the standard grid, with a soft shadow below it."""
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    body = round(size * MAC_BODY)
    offset = (size - body) // 2
    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (offset, offset + size // 100, offset + body, offset + body + size // 100),
        radius=round(body * MAC_RADIUS), fill=(0, 0, 0, 90))
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(size / 80)))
    canvas.alpha_composite(rounded(art, body, MAC_RADIUS), (offset, offset))
    return canvas


def save(image: Image.Image, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    image.save(path)


def build_apple(art: Image.Image, app: Path) -> None:
    catalog = app / "Shared/Assets.xcassets"
    icon_set = catalog / "AppIcon.appiconset"
    save(scaled(art, 1024), icon_set / "ios-1024.png")
    images = [{"filename": "ios-1024.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"}]
    for points in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            name = f"mac-{points}@{scale}x.png"
            save(mac_icon(art, points * scale), icon_set / name)
            images.append({"filename": name, "idiom": "mac", "scale": f"{scale}x", "size": f"{points}x{points}"})
    info = {"author": "xcode", "version": 1}
    (catalog / "Contents.json").write_text(json.dumps({"info": info}, indent=2))
    (icon_set / "Contents.json").write_text(json.dumps({"images": images, "info": info}, indent=2))


def adaptive_background(art: Image.Image, size: int) -> Image.Image:
    """The whole artwork inside the visible square, over a blurred copy that fills the margin."""
    backdrop = scaled(art, size).filter(ImageFilter.GaussianBlur(size / 12))
    inner = round(size * ADAPTIVE_VISIBLE)
    backdrop.paste(scaled(art, inner), ((size - inner) // 2, (size - inner) // 2))
    return backdrop


def build_android(art: Image.Image, app: Path) -> None:
    res = app / "app/src/main/res"
    for density, factor in ANDROID_DENSITIES.items():
        folder = res / f"mipmap-{density}"
        save(rounded(art, round(48 * factor), 0.18), folder / "ic_launcher.png")
        save(circle(art, round(48 * factor)), folder / "ic_launcher_round.png")
        save(adaptive_background(art, round(108 * factor)), folder / "ic_launcher_background.png")
    adaptive = (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
        '    <background android:drawable="@mipmap/ic_launcher_background" />\n'
        '    <foreground android:drawable="@android:color/transparent" />\n'
        '</adaptive-icon>\n'
    )
    anydpi = res / "mipmap-anydpi-v26"
    anydpi.mkdir(parents=True, exist_ok=True)
    for name in ("ic_launcher.xml", "ic_launcher_round.xml"):
        (anydpi / name).write_text(adaptive)


def build_web(art: Image.Image, app: Path) -> None:
    public = app / "public"
    save(rounded(art, 64, 0.22), public / "favicon.png")
    save(scaled(art, 180), public / "apple-touch-icon.png")
    save(rounded(art, 512, 0.22), public / "icon-512.png")


def build_linux(art: Image.Image, app: Path) -> None:
    icons = app / "data/icons/hicolor"
    for size in (64, 128, 256, 512):
        save(rounded(art, size, 0.2), icons / f"{size}x{size}/apps/chat.buccanet.Buccanet.png")


BUILDERS = {"apple": build_apple, "android": build_android, "web": build_web, "linux": build_linux}


def main(arguments: list[str]) -> None:
    if len(arguments) != 2 or arguments[0] not in BUILDERS:
        sys.exit(f"usage: build-icons.py <{'|'.join(BUILDERS)}> <app repository>")
    platform, app = arguments[0], Path(arguments[1]).resolve()
    BUILDERS[platform](master(), app)
    print(f"icons built for {platform} in {app}")


if __name__ == "__main__":
    main(sys.argv[1:])
