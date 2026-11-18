"""Animate the painted eye in its original head coordinates."""
import numpy as np
from PIL import Image


EYE_BOX = (528, 33, 565, 67)


def blink_amount(action, phase):
    # One brief blink while looking; close at contact on the first peck.
    if action == "walk":
        return 0.0
    start, duration, period = (0.70, 0.17, 3.4) if action == "look" else (0.465, 0.165, 2.8)
    t = (phase * period - start) / duration
    if not 0 < t < 1:
        return 0.0
    close = min(1.0, t / 0.36, (1 - t) / 0.57)
    return close * close * (3 - 2 * close)


def eye_patch(source, closure):
    original = np.array(source.crop(EYE_BOX))
    if closure <= 0:
        return Image.fromarray(original)
    x0, y0, x1, y1 = EYE_BOX
    y, x = np.mgrid[y0:y1, x0:x1]
    dx, dy = x - 546.4, y - 49
    height = 11.4 * np.sqrt(np.maximum(0, 1 - (dx / 12) ** 2))
    seam = 0.09 * dx
    inside = np.clip(height - np.abs(dy - seam) + 0.7, 0, 1) * np.clip(12.5 - np.abs(dx), 0, 1)
    lid = inside * np.clip(np.abs(dy - seam) - height * (1 - closure) + 0.5 + 0.7 * closure ** 8, 0, 1)
    feathers = np.asarray(source.crop((505, 36, 542, 70)))[..., :3].astype(float)
    feathers *= 1 - (0.50 * closure * np.exp(-((dy - seam + height * (1 - closure)) / 1.1) ** 2))[..., None]
    original[..., :3] = np.rint(original[..., :3] * (1 - lid[..., None]) + feathers * lid[..., None])
    return Image.fromarray(original)


if __name__ == "__main__":
    from pathlib import Path

    with Image.open(Path(__file__).resolve().parents[1] / "assets-2d" / "pigeon-2d-study.png") as painting:
        source = painting.crop((786, 88, 1422, 445))
        opened = np.asarray(eye_patch(source, 0))
        closed = np.asarray(eye_patch(source, 1))
        assert np.array_equal(opened, np.asarray(source.crop(EYE_BOX)))
        assert np.array_equal(opened[..., 3], closed[..., 3])
        # The eyelid covers the orange iris without leaving a bright seam at full closure.
        def iris_pixels(image):
            r, g, b = image[..., :3].astype(int).transpose(2, 0, 1)
            return np.count_nonzero((r - b > 105) & (g > 70))
        assert iris_pixels(opened) > 100 and iris_pixels(closed) < 3
