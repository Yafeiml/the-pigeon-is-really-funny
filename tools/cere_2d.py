"""Model the pale cere on the master paintings before they are animated."""
import numpy as np
from PIL import Image, ImageFilter


def shade_cere(paint, box):
    patch = np.array(paint.crop(box))
    rgb = patch[..., :3].astype(float)
    # Only the pale tissue participates. The original alpha and nostril remain intact.
    ivory = np.clip((rgb.min(axis=2) - 168) / 40, 0, 1)
    ivory *= ivory * (3 - 2 * ivory)
    mask = Image.fromarray(np.rint(ivory * patch[..., 3]).astype('uint8'))
    inside = np.asarray(mask.filter(ImageFilter.GaussianBlur(patch.shape[1] * .08))) / 255
    # A soft reflected shadow rolls off toward the outer edge even on a white desktop.
    shade = (17 + 50 * (1 - inside)) * ivory
    patch[..., :3] = np.rint(rgb - shade[..., None] * (1.04, 1.00, .92)).clip(0, 255)
    paint.paste(Image.fromarray(patch), box[:2])
