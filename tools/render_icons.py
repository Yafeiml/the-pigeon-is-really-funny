"""Export the editable SVG for Windows. Requires Pillow and resvg-py."""
from io import BytesIO
from pathlib import Path

from PIL import Image
from resvg_py import svg_to_bytes


ICONS = Path(__file__).resolve().parents[1] / "src" / "icons"
SIZES = [16, 20, 24, 28, 32, 40, 48, 64, 96, 128, 256]


def main():
    svg = (ICONS / "pigeon.svg").read_text(encoding="utf-8")
    png = svg_to_bytes(svg_string=svg, width=1024, height=1024)
    mask = Image.open(BytesIO(png)).convert("RGBA").getchannel("A")
    for name, ink in (("tray-light", "#181818"), ("tray-dark", "#f5f5f5"), ("pigeon", "#797979")):
        frames = []
        for side in SIZES:
            frame = Image.new("RGBA", (side, side), ink)
            frame.putalpha(mask.resize(frame.size, Image.Resampling.LANCZOS))
            frames.append(frame)
        frames[-1].save(ICONS / f"{name}.ico", sizes=[f.size for f in frames],
                        append_images=frames[:-1])


if __name__ == "__main__":
    main()
