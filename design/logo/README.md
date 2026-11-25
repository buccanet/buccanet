# Logo

The mark is "Two sails, mirrored": a straight jib on the left and a curved mainsail bellied to the right, drawn by hand on a 64 unit grid. The jib is the signal colour and the mainsail the main ink. Every shape is a plain path with no mask, clip, gradient or filter, so the mark also stands in one colour.

## Files

| File | Use |
|------|-----|
| `buccanet-symbol-dark.svg` | the symbol on dark grounds: chalk `#eceae4` and signal `#c6f432` |
| `buccanet-symbol-light.svg` | the symbol on light grounds: ink `#0d0f12` and dark signal `#557a00` |
| `buccanet-symbol-mono-light.svg`, `buccanet-symbol-mono-dark.svg` | the symbol in one colour: ink, then chalk |
| `buccanet-lockup-*.svg` | the symbol and the word Buccanet, in the same four palettes |
| `buccanet-favicon.svg` | the browser favicon: light colours by default, dark colours under `prefers-color-scheme: dark` |
| `buccanet-tile.svg` | the symbol at 70 % inside an ink square with 14 unit corners, for avatars and listings |
| `buccanet-icon.svg` | the source of the master icon: the tile without its rounded corners, since each platform applies its own mask |
| `buccanet-icon.png` | the master icon, `buccanet-icon.svg` rendered at 1024 px |
| `bimi.svg` | the BIMI logo of the mail domain: the ink symbol on a white square, in SVG Tiny PS |

## Scripts

`build-icons.py` renders every app icon from `buccanet-icon.png`:

```
uv run design/logo/build-icons.py <apple|android|web|linux> <app repository>
```

The Apple and Android `build-core` scripts, the Linux image, and the `icons` script of the web app and the site run it. The web target writes `favicon.png`, `apple-touch-icon.png` and `icon-512.png`, then copies `favicon.svg`, `buccanet-symbol-dark.svg` and `bimi.svg` into `public/`. Its outputs are generated and not versioned. It needs Pillow only.

`render-icon.py` renders `buccanet-icon.png` from `buccanet-icon.svg` with resvg. Run it after a change to the mark:

```
uv run design/logo/render-icon.py
```

`check-bimi.py` checks `bimi.svg` against the SVG Tiny PS rules of BIMI: `version="1.2"` and `baseProfile="tiny-ps"` on the root, a `<title>`, a square `viewBox`, no `x` or `y` on the root, the allowed elements only, no reference outside the file, and 32 KB at most. Run it after any change to `bimi.svg`:

```
python3 design/logo/check-bimi.py
```

## Explorations

`renders/` keeps the earlier concepts, unversioned because they are rendered images. The first master icon was concept 1 of those renders (a person bubble and a robot bubble meeting around a lock), rendered with GPT Image through the Codex image backend.
