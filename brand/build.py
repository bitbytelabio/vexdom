#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pillow==13.0.0"]
# ///
"""Builds every vexdom icon from the one generated source (`source-grok-1024.png`, made with the
Grok CLI: a cyberpunk X×V monogram, a thick cyan V over two thinner magenta legs with a square
node on the crossing, in HUD corner brackets on a dark tile with a faint grid and scanlines): the
macOS app icon (`AppIcon-1024.png` for `cargo run`; `AppIcon.icon` compiled to
`Assets.car` and `AppIcon.icns` for `Vexdom.app`, scripts/bundle-mac.sh), the menu-bar template
(`MenuBarTemplate.png`, `@2x`) and the extension's icons (`packages/extension/.../public/icon`).

Run from anywhere: `uv run brand/build.py`. Needs Xcode 26 or later (`xcrun actool` reads Icon
Composer icons).
"""

import json
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

HERE = Path(__file__).resolve().parent
EXTENSION_ICONS = HERE.parent / "packages/extension/src/extension/public/icon"

# The generated tile fills the canvas.
TILE_BOX = (0, 0, 1024, 1024)
# Apple's icon grid: an 824 px tile on a 1024 px canvas, ~185 px corners, a soft drop shadow.
TILE, CANVAS, RADIUS = 824, 1024, 185


def rounded_mask(size: int, radius: int, scale: int = 4) -> Image.Image:
    big = Image.new("L", (size * scale, size * scale), 0)
    ImageDraw.Draw(big).rounded_rectangle(
        (0, 0, size * scale - 1, size * scale - 1), radius * scale, fill=255
    )
    return big.resize((size, size), Image.LANCZOS)


def app_icon(tile: Image.Image) -> Image.Image:
    mask = rounded_mask(TILE, RADIUS)
    offset = (CANVAS - TILE) // 2
    shadow = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    shadow.paste((0, 0, 0, 77), (offset, offset + 12), mask)
    canvas = shadow.filter(ImageFilter.GaussianBlur(14))
    layer = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    layer.paste(tile, (offset, offset), mask)
    return Image.alpha_composite(canvas, layer)


def app_bundle_icon(source: Image.Image) -> None:
    """`Vexdom.app`'s icon. macOS 26 and later set a plain `.icns` on a grey plate of its own; an
    Icon Composer icon compiled into `Assets.car` (`CFBundleIconName`) is masked to the system
    shape instead, so the tile goes edge to edge and macOS rounds it. `actool` also
    writes the `AppIcon.icns` older macOS (`CFBundleIconFile`) shows."""
    icon = HERE / "AppIcon.icon"
    shutil.rmtree(icon, ignore_errors=True)
    (icon / "Assets").mkdir(parents=True)
    source.crop(TILE_BOX).resize((CANVAS, CANVAS), Image.LANCZOS).save(icon / "Assets/plate.png")
    (icon / "icon.json").write_text(json.dumps(ICON_JSON, indent=2) + "\n")
    with tempfile.TemporaryDirectory() as work:
        out = Path(work) / "out"
        out.mkdir()
        subprocess.run(
            [
                "xcrun", "actool", str(icon), "--compile", str(out),
                "--platform", "macosx", "--minimum-deployment-target", "14.0",
                "--app-icon", "AppIcon",
                "--output-partial-info-plist", str(Path(work) / "partial.plist"),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        for name in ("Assets.car", "AppIcon.icns"):
            shutil.copyfile(out / name, HERE / name)


# One layer, no glass: the picture already has its own light and depth.
ICON_JSON = {
    "fill": "automatic",
    "groups": [
        {
            "layers": [{"image-name": "plate.png", "name": "plate"}],
            "shadow": {"kind": "neutral", "opacity": 0.5},
            "translucency": {"enabled": False, "value": 0.5},
        }
    ],
    "supported-platforms": {"squares": "shared"},
}


def menu_bar_template(px: int) -> Image.Image:
    """The mark (a V over two legs, an X), black on clear: macOS tints a template."""
    scale = 16
    big = Image.new("RGBA", (px * scale, px * scale), (0, 0, 0, 0))
    draw = ImageDraw.Draw(big)
    unit = px * scale / 18.0
    P = lambda x, y: (x * unit, y * unit)  # noqa: E731

    draw.line([P(2, 2.5), P(9, 10.5), P(16, 2.5)], fill=(0, 0, 0, 255), width=round(2.2 * unit), joint="curve")
    draw.line([P(9, 10.5), P(13.2, 15.3)], fill=(0, 0, 0, 255), width=round(1.5 * unit))
    draw.line([P(9, 10.5), P(4.8, 15.3)], fill=(0, 0, 0, 255), width=round(1.5 * unit))
    return big.resize((px, px), Image.LANCZOS)


def main() -> None:
    source = Image.open(HERE / "source-grok-1024.png").convert("RGB")
    tile = source.crop(TILE_BOX).resize((TILE, TILE), Image.LANCZOS)

    # The Dock icon of `cargo run` (set at run time, shown as drawn).
    app_icon(tile).save(HERE / "AppIcon-1024.png")
    app_bundle_icon(source)

    menu_bar_template(18).save(HERE / "MenuBarTemplate.png")
    menu_bar_template(36).save(HERE / "MenuBarTemplate@2x.png")

    # The extension's toolbar icon: the tile edge to edge, its own corners rounded.
    flat = Image.new("RGBA", (TILE, TILE), (0, 0, 0, 0))
    flat.paste(tile, (0, 0), rounded_mask(TILE, RADIUS))
    EXTENSION_ICONS.mkdir(parents=True, exist_ok=True)
    for px in (16, 32, 48, 96, 128):
        flat.resize((px, px), Image.LANCZOS).save(EXTENSION_ICONS / f"{px}.png")


if __name__ == "__main__":
    if subprocess.run(["xcrun", "--find", "actool"], capture_output=True).returncode != 0:
        raise SystemExit("Xcode's actool is needed for Assets.car and AppIcon.icns")
    main()
