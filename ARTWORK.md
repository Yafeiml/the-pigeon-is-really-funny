# Pigeon artwork

English · [简体中文](ARTWORK.zh-CN.md)

The project distributes this character artwork and the animation scripts under the MIT License. Please keep the copyright and permission notices with redistributed copies. [Read the license](LICENSE)

| Asset | How it was made and used |
| --- | --- |
| `assets-2d/pigeon-2d-study.png` | The project created this character sheet with an image generation tool. It contains standing, walking and pecking poses. Two images of a pigeon model served as references for anatomy and plumage. |
| `assets-2d/rig/turn-views.png` | Three-quarter and front views of the same pigeon provide the key poses for turning. |
| Windows animation | The scripts use the two sheets to render 2,508 frames across the walk, low walk, settling, turning, looking and pecking sequences. |
| [Pigeon icon](src/icons/pigeon.svg) | A hand-drawn front view of the head, beak and shoulders. The tray uses dark or light ink on a transparent background to match the taskbar. Explorer uses a neutral gray version. |

Footage of real pigeons informed the gait and foraging animation. Rights to the reference photos, gallery images, model previews, videos and video stills remain with their respective owners. Those files are excluded from the source and executable. The animation can be rebuilt using the two sheets above.

After editing the SVG, run `python tools/render_icons.py` with Pillow and resvg-py to regenerate the three Windows icon files.
