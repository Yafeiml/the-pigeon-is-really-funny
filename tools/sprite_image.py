from PIL import Image


def resize_rgba(image, size):
    # Resample premultiplied pixels so transparent edges keep their feather colour.
    return image.convert("RGBa").resize(size, Image.Resampling.LANCZOS).convert("RGBA")
