#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Draws Baton's app icon at every size that the iOS and macOS projects ask for.

Two dots with a baton between them: one device handing over to another. Run this from
apps/baton after changing the drawing below, and commit the pictures it writes:

    python3 tool/make_icons.py

It needs Pillow (python3 -m pip install pillow). The pictures are what the apps use; nothing
at build time runs this.
"""

import math
import pathlib

from PIL import Image, ImageDraw, ImageFilter

HERE = pathlib.Path(__file__).resolve().parent.parent

TOP_LEFT = (0x36, 0x4F, 0xC7)
BOTTOM_RIGHT = (0x5C, 0x7C, 0xFA)

# iOS asks for each size of picture by name; the names are in
# ios/Runner/Assets.xcassets/AppIcon.appiconset/Contents.json.
IOS_ICONS = {
    "Icon-App-20x20@1x.png": 20,
    "Icon-App-20x20@2x.png": 40,
    "Icon-App-20x20@3x.png": 60,
    "Icon-App-29x29@1x.png": 29,
    "Icon-App-29x29@2x.png": 58,
    "Icon-App-29x29@3x.png": 87,
    "Icon-App-40x40@1x.png": 40,
    "Icon-App-40x40@2x.png": 80,
    "Icon-App-40x40@3x.png": 120,
    "Icon-App-60x60@2x.png": 120,
    "Icon-App-60x60@3x.png": 180,
    "Icon-App-76x76@1x.png": 76,
    "Icon-App-76x76@2x.png": 152,
    "Icon-App-83.5x83.5@2x.png": 167,
    "Icon-App-1024x1024@1x.png": 1024,
}

# The same for macOS, in macos/Runner/Assets.xcassets/AppIcon.appiconset/Contents.json.
MACOS_ICONS = {f"app_icon_{size}.png": size for size in (16, 32, 64, 128, 256, 512, 1024)}


def gradient(size):
    """A diagonal gradient, made small and scaled up, which is smooth and cheap."""
    n = 256
    small = Image.new("RGB", (n, n))
    pixels = small.load()
    for y in range(n):
        for x in range(n):
            t = (x + y) / (2 * (n - 1))
            pixels[x, y] = tuple(
                round(a + (b - a) * t) for a, b in zip(TOP_LEFT, BOTTOM_RIGHT)
            )
    return small.resize((size, size), Image.BICUBIC)


def motif(size):
    """The two dots and the baton, white on black, as a mask of `size` pixels square."""
    scale = 4  # drawn large and shrunk, so the edges are smooth
    big = size * scale
    mask = Image.new("L", (big, big), 0)
    draw = ImageDraw.Draw(mask)

    def at(u, v):
        return (u * big, v * big)

    def dot(centre, radius):
        x, y = at(*centre)
        r = radius * big
        draw.ellipse((x - r, y - r, x + r, y + r), fill=255)

    dot((0.235, 0.765), 0.09)
    dot((0.765, 0.235), 0.09)

    # The baton is a rectangle with a half circle on each end, so that it is exactly as wide
    # along its length as at its ends.
    start, end, radius = (0.385, 0.615), (0.615, 0.385), 0.085
    (x1, y1), (x2, y2) = at(*start), at(*end)
    length = math.hypot(x2 - x1, y2 - y1)
    nx, ny = (y1 - y2) / length * radius * big, (x2 - x1) / length * radius * big
    draw.polygon(
        [(x1 + nx, y1 + ny), (x2 + nx, y2 + ny), (x2 - nx, y2 - ny), (x1 - nx, y1 - ny)],
        fill=255,
    )
    dot(start, radius)
    dot(end, radius)
    return mask.resize((size, size), Image.LANCZOS)


def ios_master():
    """The picture iOS rounds the corners of itself: square, edge to edge, no transparency."""
    size = 1024
    white = Image.new("RGB", (size, size), (255, 255, 255))
    return Image.composite(white, gradient(size), motif(size))


def macos_master():
    """A Mac shows the picture as it is, so this one has rounded corners and a shadow."""
    size, art_size, radius = 1024, 824, 185
    margin = (size - art_size) // 2
    white = Image.new("RGB", (art_size, art_size), (255, 255, 255))
    art = Image.composite(white, gradient(art_size), motif(art_size))

    scale = 4
    corners = Image.new("L", (art_size * scale, art_size * scale), 0)
    ImageDraw.Draw(corners).rounded_rectangle(
        (0, 0, art_size * scale - 1, art_size * scale - 1), radius * scale, fill=255
    )
    corners = corners.resize((art_size, art_size), Image.LANCZOS)

    shadow = Image.new("L", (size, size), 0)
    shadow.paste(corners, (margin, margin + 12))
    shadow = shadow.filter(ImageFilter.GaussianBlur(14)).point(lambda a: a * 90 // 255)

    icon = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    icon.paste(Image.new("RGBA", (size, size), (0, 0, 0, 255)), mask=shadow)
    icon.paste(art, (margin, margin), mask=corners)
    return icon


def write(master, folder, names):
    folder.mkdir(parents=True, exist_ok=True)
    for name, size in names.items():
        picture = master if size == master.width else master.resize((size, size), Image.LANCZOS)
        picture.save(folder / name, optimize=True)


if __name__ == "__main__":
    write(ios_master(), HERE / "ios/Runner/Assets.xcassets/AppIcon.appiconset", IOS_ICONS)
    write(macos_master(), HERE / "macos/Runner/Assets.xcassets/AppIcon.appiconset", MACOS_ICONS)
