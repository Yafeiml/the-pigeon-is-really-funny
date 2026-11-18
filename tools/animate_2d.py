"""Animate the original walking and pecking paintings over one planted-foot leg rig."""
import argparse
import hashlib
import json
import math
import os
import subprocess
import tempfile
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter

from sprite_image import resize_rgba
from cere_2d import shade_cere
from pigeon_blink import EYE_BOX, blink_amount, eye_patch
from peck_plumage import key_point, key_weight, painted_peck, prepare_key
from turn_2d import TURN_FRAMES, bake as bake_turn

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "assets-2d"
COUNTS = {"walk": 256, "look": 1020, "peck": 840, "stalk": 256, "settle": 56}
SIZE = (512, 408)
SUPER = 2
SCALE = 0.38
PIVOT = np.array((450.0, 760.0))
SOURCE = ASSETS / "pigeon-2d-study.png"


def smooth(value):
    value = np.clip(value, 0, 1)
    return value * value * (3 - 2 * value)


def rotation(angle):
    return np.array(((math.cos(angle), -math.sin(angle)), (math.sin(angle), math.cos(angle))))


def frame_point(point):
    return (np.asarray(point) - (450, 970)) * SCALE + (272, 354)


def mask_image(source, polygon, feather=1.1):
    mask = Image.new("L", source.size)
    ImageDraw.Draw(mask).polygon(polygon, fill=255)
    mask = mask.filter(ImageFilter.GaussianBlur(feather))
    result = source.copy()
    result.putalpha(ImageChops.multiply(source.getchannel("A"), mask))
    return result


def texture(image):
    return np.asarray(image.convert("RGBa"), dtype=np.float32)


def triangle(canvas, pixels, uv, xy):
    xy = np.asarray(xy) * SUPER
    uv = np.asarray(uv)
    lo = np.maximum(np.floor(xy.min(axis=0)).astype(int), 0)
    hi = np.minimum(np.ceil(xy.max(axis=0)).astype(int) + 1, canvas.shape[1::-1])
    if np.any(hi <= lo):
        return
    matrix = np.column_stack((xy[1] - xy[0], xy[2] - xy[0]))
    determinant = np.linalg.det(matrix)
    if abs(determinant) < 1e-7:
        return
    yy, xx = np.mgrid[lo[1]:hi[1], lo[0]:hi[0]]
    points = np.stack((xx + 0.5, yy + 0.5), axis=-1)
    bary = (points - xy[0]) @ np.linalg.inv(matrix).T
    weights = np.stack((1 - bary.sum(axis=-1), bary[..., 0], bary[..., 1]), axis=-1)
    inside = weights.min(axis=-1) >= -1e-6
    if not inside.any():
        return
    sample = weights[inside] @ uv
    sx, sy = np.clip(sample[:, 0], 0, pixels.shape[1] - 1.001), np.clip(sample[:, 1], 0, pixels.shape[0] - 1.001)
    ix, iy = sx.astype(int), sy.astype(int)
    fx, fy = (sx - ix)[:, None], (sy - iy)[:, None]
    color = (pixels[iy, ix] * (1 - fx) + pixels[iy, ix + 1] * fx) * (1 - fy) + (pixels[iy + 1, ix] * (1 - fx) + pixels[iy + 1, ix + 1] * fx) * fy
    view = canvas[lo[1]:hi[1], lo[0]:hi[0]]
    # A mesh is one surface: shared triangle edges overwrite rather than accumulate alpha.
    view[inside] = color


def quad(canvas, pixels, uv, xy):
    for indices in ((0, 1, 2), (0, 2, 3)):
        triangle(canvas, pixels, [uv[i] for i in indices], [xy[i] for i in indices])


def layer_frame(draw):
    layer = np.zeros((SIZE[1] * SUPER, SIZE[0] * SUPER, 4), dtype=np.float32)
    draw(layer)
    return Image.fromarray(np.clip(layer, 0, 255).astype(np.uint8), "RGBa").convert("RGBA")


def round_belly(plumage, ends, depth):
    # Follow the underside of the feather layer before the separate legs occlude it.
    left, right = ends * SUPER
    x0, x1 = max(0, math.ceil(left)), min(plumage.width, math.ceil(right))
    if x0 >= x1:
        return
    alpha = np.asarray(plumage.getchannel("A").crop((x0, 0, x1, plumage.height)))
    covered = alpha >= 128
    edge = plumage.height - 1 - np.argmax(covered[::-1], axis=0)
    active = covered.any(axis=0)
    if not active.any():
        return
    u = (np.arange(x0, x1) - left) / (right - left)
    # The deepest part sits just in front of the thigh, tapering over the longer rise to the breast.
    fullness = (3 * u)**2 * (1.5 * (1 - u))**4
    columns = np.arange(x1 - x0)
    inside = alpha[edge, columns].astype(float)
    outside = alpha[np.minimum(edge + 1, plumage.height - 1), columns].astype(float)
    outline = edge + (inside - 127.5) / np.maximum(inside - outside, 1)
    radius = 12 * SUPER
    kernel = np.exp(-0.5 * (np.arange(-radius, radius + 1) / (5 * SUPER))**2)
    rounded = np.convolve(np.pad(outline, radius, mode="edge"), kernel / kernel.sum(), mode="valid")
    extension = fullness * (depth * SUPER + np.clip(rounded - outline, -2 * SUPER, 2 * SUPER)) * active
    y0 = max(0, int(edge[active].min() - 32 * SUPER))
    y1 = min(plumage.height, math.ceil((edge + extension)[active].max()) + 5 * SUPER)
    band = plumage.crop((x0, y0, x1, y1))
    y = np.arange(y0, y1)[:, None]
    shift = extension * smooth((y - edge + 30 * SUPER) / (24 * SUPER))
    source_y = np.clip(y - y0 - shift, 0, band.height - 1)
    upper = source_y.astype(int)
    mix = (source_y - upper)[..., None]
    pixels = np.asarray(band.convert("RGBa"))
    x = np.arange(band.width)
    moved = pixels[upper, x] * (1 - mix) + pixels[np.minimum(upper + 1, band.height - 1), x] * mix
    colors = Image.fromarray(np.rint(moved).astype("uint8"), "RGBa").convert("RGBA")
    plumage.paste(colors, (x0, y0), Image.fromarray((shift > 0.001).astype("uint8") * 255))


def folded_leg(knee, ankle, thigh, shank):
    """Keep both bone lengths and place the hock behind the knee in side view."""
    axis = ankle - knee
    span = max(float(np.linalg.norm(axis)), 0.001)
    direction = axis / span
    # Let the knee inside the plumage yield before full extension so the hock never snaps straight.
    full = thigh + shank
    soft = full * 0.10
    onset = full - soft
    reach = span if span <= onset else onset - soft * math.expm1(-(span - onset) / soft)
    reach = np.clip(reach, abs(thigh - shank) + 0.01, full - 0.01)
    knee = ankle - direction * reach
    along = (thigh * thigh - shank * shank + reach * reach) / (2 * reach)
    back = np.array((-direction[1], direction[0]))
    return knee, knee + direction * along + back * math.sqrt(max(0, thigh * thigh - along * along))


class Rig:
    def __init__(self):
        with Image.open(SOURCE) as opened:
            original = opened.convert("RGBA")
        assert original.size == (1448, 1086), "Rig coordinates refer to the approved original canvas"
        shade_cere(original, (1359, 131, 1395, 161))
        shade_cere(original, (1308, 888, 1344, 924))
        self.peck_key = prepare_key(original)
        source = Image.new("RGBA", original.size)
        source.paste(original.crop((0, 60, 790, 1010)), (0, 60))
        # Discard the one-pixel matte fringe before the drawing is reduced to desktop size.
        alpha = source.getchannel("A").filter(ImageFilter.MinFilter(3))
        source.putalpha(alpha)
        # The top-right painting already has the low shoulder and oblique throat of a walking bird.
        self.walk_origin = np.array((786, 88))
        walk = original.crop((*self.walk_origin, 1422, 445))
        lower_contour = [(1422,444), (1180,381), (1155,387), (1110,396),
                         (1060,401), (1020,401), (983,394), (960,391), (949,445), (786,445)]
        self.walk_skin = mask_image(walk, [tuple(np.asarray(p) - self.walk_origin) for p in
            [(786,88), (1422,88), *lower_contour]], feather=1.2)
        self.walk_skin.putalpha(ImageChops.multiply(self.walk_skin.getchannel("A"),
                                walk.getchannel("A").filter(ImageFilter.MinFilter(3))))
        self.walk_pixels = texture(self.walk_skin)
        self.sections = []
        xs = (786, 970, 1100, 1140, 1170, 1192, 1215, 1238, 1260, 1271, 1281, 1292,
              1304, 1314, 1324, 1334, 1344, 1354, 1364, 1375, 1386, 1402, 1422)
        ys = (88, 127, 154, 162, 170, 178, 186, 194, 202, 212, 220, 227, 239, 251, 277, 308, 345, 385, 445)
        for x0, x1 in zip(xs, xs[1:]):
            for y0, y1 in zip(ys, ys[1:]):
                uv = np.array([(x0,y0), (x1,y0), (x1,y1), (x0,y1)]) - self.walk_origin
                if self.walk_skin.crop((*uv[0], *uv[2])).getbbox():
                    self.sections.append((uv, uv + self.walk_origin))
        # Both moving legs take their cuff from the walking painting. Its underside
        # contains the rounded feather tips, with no second leg or belly seam.
        cuff_box = (995, 345, 1160, 445)
        outline = [(1010,345),(1158,345),(1156,377),(1138,391),(1106,402),
                   (1096,409),(1084,417),(1059,429),(1045,433),(1034,435),
                   (1026,435),(1019,433),(1013,429),(1011,424),(1012,416),
                   (1018,405),(1018,399),(1009,390)]
        cuff = mask_image(original.crop(cuff_box),
                          [(x-cuff_box[0], y-cuff_box[1]) for x,y in outline], 0.75)
        cuff.putalpha(cuff.getchannel("A").point(lambda a: max(0, min(255, (a-20)*1.2))))
        self.legs = []
        for far in (True, False):
            toes_poly, knee, hock, ankle = (
                ([(446,824),(511,825),(540,882),(650,909),(655,968),(501,969),(481,951),(444,937),(432,901)],
                 (522,734), (474,837), (501,904))
                if far else
                ([(337,833),(404,837),(410,902),(424,937),(481,950),(481,997),(276,1005),(277,940),(332,902)],
                 (419,741), (370,847), (359,931)))
            thigh = cuff.copy()
            lower = mask_image(source, toes_poly)
            if far:
                # The far limb is slightly darker throughout, rather than switching depth at crossover.
                for part in (thigh, lower):
                    rgb = np.asarray(part).copy()
                    rgb[..., :3] = np.round(rgb[..., :3] * (0.78, 0.80, 0.86))
                    part.paste(Image.fromarray(rgb))
            self.legs.append((far, texture(thigh), texture(lower),
                              *[np.array(v, float) for v in (knee, hock, ankle)]))

    def render(self, pose, action, phase, low=0.0):
        walking = action in ("walk", "stalk", "settle")
        if action == "stalk":
            low = 1.0
        peck = pose["peck"]
        attention = abs(pose["look"]) if action == "look" else 0
        body_pivot = frame_point(PIVOT)
        body_angle = math.radians(45 * peck + (12 - 4 * attention + 3 * low) * (1 - peck)) + pose["body_pitch"] * (1 - peck)
        body_rotation = rotation(body_angle)
        body_shift = np.array((-18 * peck, pose["body_y"] * 2 - 10 * peck + (3 + 3 * low) * (1 - peck)))

        def body_point(point):
            return (frame_point(point) - body_pivot) @ body_rotation.T + body_pivot + body_shift

        # Map the original walking painting into the same size and foot space as the leg rig.
        scale = 0.61
        pivot = np.array((1072., 355.))
        anchor = np.array((271., 251.))
        eye = np.array((1332., 137.))
        torso_pitch = math.radians(-4 + 6 * low - 4 * attention + 44 * peck) + pose["body_pitch"] * (1 - peck)
        torso_matrix = rotation(torso_pitch)
        skull = rotation(math.radians(-3 + 12 * low + 66 * peck - 8 * pose["look"]))
        eye_rest = (eye - pivot) * scale + anchor
        stationary = 0.0 if walking else 8.0
        eye_center = eye_rest + np.array((-23 + 11 * low - 16 * attention + stationary + (pose["head_x"] - 48 - 30 * peck) * 2,
                          -10 + 32 * low - 34 * attention + (pose["head_y"] + 61 - 108 * peck) * 2))
        if not walking:
            eye_center[1] += pose["body_y"] * 2
        eye_center = eye_center * (1 - peck) + key_point((1307,864)) * peck

        def walk_point(points):
            pts = np.asarray(points, float)
            x, y = pts[..., 0], pts[..., 1]
            # Spread the throat feathers down into the crop, with the wing and bill held in place.
            throat = (smooth((x - 1160) / 125) * smooth((1385 - x) / 52)
                      * smooth((y - 153) / 55) * np.exp(-((y - 245) / 91) ** 2))
            nape = (np.exp(-((x - 1213) / 81) ** 2 - ((y - 167) / 45) ** 2)
                    * (1 - smooth((x - 1240) / 53)) * (1 - smooth((y - 176) / 34)))
            plumage = pts + (1 - smooth(peck)) * np.stack((32 * throat - 4 * nape,
                                                            11 * throat - 12 * nape), axis=-1)
            body = (plumage - pivot) * scale @ torso_matrix.T + anchor + body_shift
            head = (plumage - eye) * scale @ skull.T + eye_center
            # The flight feathers move rigidly. Only the feathered neck between shoulder and crown flexes.
            weight = smooth(((pts[...,0]-1155)*0.65 + (240-pts[...,1])*0.60) / 139)[...,None]
            return body * (1 - weight) + head * weight

        canvas = Image.new("RGBA", (SIZE[0] * SUPER, SIZE[1] * SUPER))
        for far, thigh_texture, lower_texture, knee, hock, ankle in self.legs:
            name = "far" if far else "near"
            foot_scale = SCALE * (0.78 if far else 0.83)
            cycle = (phase + (0.5 if far else 0)) % 1
            swing = max(0.0, (cycle - 0.64) / 0.36) if walking else 0.0
            tuck = math.sin(math.pi * swing) ** 2
            pitch = math.radians(22) * math.sin(2 * math.pi * swing) * math.sin(math.pi * swing)
            ground_y = 346 if far else 354
            rest_depth = (965 if far else 994) - ankle[1]
            target_ankle = np.array((272 + pose[f"{name}_foot_x"] * 2 + (-8 if far else 3),
                                     ground_y + (pose[f"{name}_foot_y"] - 72) * 2 - rest_depth * foot_scale))
            target_knee = body_point((438, 737) if far else (438, 743))
            if peck:
                keyed = smooth((peck - 0.29) / 0.69)
                folded_knee = np.array((268, 267) if far else (307, 269))
                target_knee = target_knee * (1 - keyed) + folded_knee * keyed
            thigh_length = np.linalg.norm(hock - knee) * SCALE
            shin_length = np.linalg.norm(ankle - hock) * SCALE
            target_knee, target_hock = folded_leg(target_knee, target_ankle, thigh_length, shin_length)

            def draw_cuff(layer):
                # Tether the upper plumage beneath the belly. The lower end follows
                # the hock; the bend is absorbed across the feather surface.
                root = target_knee + (-2, -11)
                # The far painting exposes its first pink scales farther below the joint.
                tip = target_hock + (target_ankle - target_hock) / shin_length * (7.5 if far else 2)
                rows = (0,13,25,38,49,60,68,77,85,92,99)
                for y0,y1 in zip(rows, rows[1:]):
                    uv = np.array([(0,y0),(164,y0),(164,y1),(0,y1)])
                    t = ((uv[:,1]-25)/65)[:,None]
                    xy = root + (tip-root)*t
                    original_center = 1095 + (1025-1095)*t[:,0]
                    xy[:,0] += (995 + uv[:,0] - original_center) * (0.35 if far else 0.50)
                    quad(layer, thigh_texture, uv, xy)

            # Shank starts under the feather cuff. Draw the rigid thigh over its upper edge.
            def draw_shank(layer):
                delta = target_ankle - target_hock
                old = ankle - hock
                tilt = math.atan2(delta[1],delta[0]) - math.atan2(old[1],old[0])
                # Orient the last few scales along the toes so the joint has no diagonal cut.
                def section(t):
                    src = hock + t * old
                    center = target_hock + t * delta
                    blend = smooth((t - 0.62) / 0.38)
                    angle = tilt * (1-blend) + pitch * blend
                    # The far shank emerges at its painted width beneath the feather tips.
                    root_round = 1.0 if far else 0.33 + 0.67 * smooth((t + 0.015) / 0.16)
                    radius = 26 * root_round * ((1-blend)*SCALE + blend*foot_scale)
                    v = radius * np.array((math.cos(angle),math.sin(angle)))
                    return [src - (26,0),src + (26,0)], [center-v,center+v]
                sections = [section(t) for t in (-0.015,0.08,0.16,0.36,0.59,0.76,0.88,1.0,1.034)]
                for (uv0,xy0),(uv1,xy1) in zip(sections,sections[1:]):
                    quad(layer,lower_texture,[*uv0,*reversed(uv1)],[*xy0,*reversed(xy1)])
            canvas.alpha_composite(layer_frame(draw_shank))
            cuff = layer_frame(draw_cuff)
            canvas.alpha_composite(cuff)
            if not far:
                overlap_box = cuff.getbbox()
                near_feathers = cuff.crop(overlap_box).getchannel("A")

            def draw_toes(layer):
                # Shorten distal digits in swing while leaving the ankle socket stationary.
                def toe_at(x,y):
                    dx, dy = x - ankle[0], y - ankle[1]
                    bend = smooth((abs(dx) + max(0,dy) - 11) / 65) * tuck
                    local = np.array((dx * (1 - 0.53*bend), dy * (1 - 0.24*bend) + 0.19*abs(dx)*bend))
                    return target_ankle + (local * foot_scale) @ rotation(pitch).T
                ys = [ankle[1]-7, ankle[1]+7, ankle[1]+28, ankle[1]+75]
                xs = [ankle[0]+v for v in (-83,-30,0,32,66,100,150)]
                for x0,x1 in zip(xs,xs[1:]):
                    for y0,y1 in zip(ys,ys[1:]):
                        uv = [(x0,y0),(x1,y0),(x1,y1),(x0,y1)]
                        quad(layer,lower_texture,uv,[toe_at(*q) for q in uv])
            canvas.alpha_composite(layer_frame(draw_toes))

        closure = blink_amount(action, phase) if not walking else 0.0
        pixels = self.walk_pixels
        if closure > 0:
            opened = self.walk_skin.copy()
            opened.paste(eye_patch(self.walk_skin, closure), EYE_BOX[:2])
            pixels = texture(opened)

        def draw_plumage(layer):
            for uv, source in self.sections:
                xy = walk_point(source)
                edges = xy - np.roll(xy, 1, axis=0)
                following = np.roll(edges, -1, axis=0)
                assert np.all(edges[:,0]*following[:,1] - edges[:,1]*following[:,0] > 0), (action, phase, "folded neck surface")
                quad(layer, pixels, uv, xy)

        plumage = layer_frame(draw_plumage)
        if peck:
            marks = walk_point([(907,361), (984,310), (1040,276), (1140,221), eye])[:, 0]
            plumage = painted_peck(plumage, self.peck_key, peck, closure, marks, SUPER)
        bow = key_weight(peck)
        belly_ends = walk_point([(1040, 399), (1290, 255)])[:, 0] * (1 - bow)
        belly_ends += key_point([(998, 854), (1190, 859)])[:, 0] * bow
        round_belly(plumage, belly_ends, 7.2 * (1 - bow) + 4.6 * bow)
        # Blend only where the near plumage fills the silhouette. A lifted thigh
        # can otherwise reveal the straight cut used to isolate the belly painting.
        belly = plumage.crop(overlap_box)
        alpha = belly.getchannel("A")
        inner = alpha.filter(ImageFilter.MinFilter(11)).filter(ImageFilter.GaussianBlur(6))
        overlap = near_feathers.point(lambda a: max(0, (a-195)*255//60))
        belly.putalpha(Image.composite(ImageChops.darker(alpha, inner), alpha, overlap))
        plumage.paste(belly, overlap_box)
        canvas.alpha_composite(plumage)
        result = resize_rgba(canvas, SIZE)
        return result


def check_frames(output=ASSETS):
    report = {}
    for action, count in {**COUNTS, "turn": TURN_FRAMES}.items():
        directory = output / action
        paths = sorted(directory.glob("*.png"))
        assert len(paths) == count, (action, len(paths), count)
        unique, boxes, byte_count = set(), [], 0
        for index, path in enumerate(paths):
            assert path.name == f"{index:04d}.png"
            with Image.open(path) as image:
                assert image.mode == "RGBA" and image.size == SIZE, path
                alpha = image.getchannel("A")
                assert alpha.getextrema() == (0, 255), path
                assert sum(alpha.histogram()[1:255]) > 0, path
                for box in ((0, 0, 512, 2), (0, 406, 512, 408), (0, 0, 2, 408), (510, 0, 512, 408)):
                    assert alpha.crop(box).getextrema() == (0, 0), (path, "Clipped border")
                unique.add(hashlib.sha256(image.tobytes()).hexdigest())
                boxes.append(alpha.getbbox())
            byte_count += path.stat().st_size
        # Still observation and ground contact may legitimately share the exact same drawing.
        assert len(unique) > count * (0.45 if action == "peck" else 0.85), (action, "Too few distinct poses")
        report[action] = {"frames": count, "unique_frames": len(unique), "bytes": byte_count, "bounds": [min(box[i] for box in boxes) if i < 2 else max(box[i] for box in boxes) for i in range(4)]}
    for gait, endpoint in (("walk", 0), ("stalk", COUNTS["settle"] - 1)):
        with Image.open(output / "settle" / f"{endpoint:04}.png") as settled, Image.open(output / gait / f"{COUNTS['walk']//8:04}.png") as stride:
            assert settled.tobytes() == stride.tobytes(), f"{gait} and settle disagree at double support"
    with Image.open(output / "settle/0000.png") as standing, Image.open(output / "turn/0000.png") as side:
        assert standing.tobytes() == side.tobytes(), "Turning must start in the planted side view"
    print(json.dumps(report))
    return report


def write_validation(output):
    report = {"source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
              "painted_poses": ["walking master", "upright leg rig", "registered peck key"],
              "frame_size": SIZE, "clips": check_frames(output)}
    (output / "animation-validation.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


def self_check():
    test = np.full((6, 6, 4), 255, dtype=np.float32)
    layer = np.zeros((12, 12, 4), dtype=np.float32)
    quad(layer, test, [(0, 0), (5, 0), (5, 5), (0, 5)], [(0, 0), (5, 0), (5, 5), (0, 5)])
    assert np.all(layer[:10, :10, 3] == 255), "A triangulated rectangle must not have diagonal cracks"
    np.testing.assert_allclose(frame_point((450, 970)), (272, 354))
    cere = Image.new("RGBA", (30, 24), (93, 84, 99, 255))
    ImageDraw.Draw(cere).ellipse((6, 4, 24, 20), fill=(249, 244, 236, 250))
    cere_alpha = cere.getchannel("A").tobytes()
    shade_cere(cere, (3, 1, 27, 23))
    assert cere.getchannel("A").tobytes() == cere_alpha and cere.getpixel((3, 3)) == (93, 84, 99, 255)
    assert cere.getpixel((15, 4))[0] < cere.getpixel((15, 12))[0] < 240
    torso = Image.new("RGBA", (132, 140))
    ImageDraw.Draw(torso).rectangle((8, 10, 122, 104), fill=(101, 91, 113, 255))
    before = torso.getchannel("A")
    round_belly(torso, np.array([10., 58.]), 4.)
    after = torso.getchannel("A")
    assert before.crop((0, 0, 132, 42)).tobytes() == after.crop((0, 0, 132, 42)).tobytes()
    assert after.getpixel((52, 110)) == 255 and after.getpixel((20, 110)) == 0
    # Swing, weight-bearing and fully extended legs must preserve both painted bone lengths.
    ankle = np.array((24.0, 64.0))
    for hip in ((0, 0), (12, 38), (-38, -24)):
        knee, hock = folded_leg(np.array(hip, float), ankle, 46.0, 31.0)
        np.testing.assert_allclose([np.linalg.norm(knee - hock), np.linalg.norm(hock - ankle)], [46, 31])
    # Crossing full reach in quarter-pixel increments must not introduce a visible kink.
    joints = [folded_leg(np.array((0., y)), np.zeros(2), 46., 31.)[1]
              for y in np.arange(72., 82., .25)]
    assert np.linalg.norm(np.diff(joints, n=2, axis=0), axis=1).max() < .03


def init_worker(clips, output):
    global worker_rig, worker_clips, worker_output
    worker_rig, worker_clips, worker_output = Rig(), clips, output


def bake_frame(job):
    action, index = job
    base = "walk" if action in ("stalk", "settle") else action
    data = worker_clips[base]
    source_index = COUNTS["walk"] // 8 if action == "settle" else index
    pose = data["poses"][source_index].copy()
    phase = pose.get("time_seconds", data["seconds"] * source_index / len(data["poses"])) / data["seconds"]
    if base != "walk":
        # Close the idle/breathing loop instead of resetting at an unrelated pose.
        closing = smooth((phase - 0.9) / 0.1)
        for name in ("body_y", "look"):
            pose[name] *= 1 - closing
    low = smooth(index / (COUNTS[action] - 1)) if action == "settle" else 0
    frame = worker_rig.render(pose, action, phase, low)
    frame.save(worker_output / action / f"{index:04d}.png", compress_level=6)
    return action


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--samples", action="store_true")
    parser.add_argument("--clip", choices=COUNTS, help="Rebake one clip when the other pose tables are current")
    parser.add_argument("--out", type=Path, default=ASSETS, help="Write generated assets to this directory")
    parser.add_argument("--jobs", type=int, default=min(8, os.cpu_count() or 1))
    arguments = parser.parse_args()
    if arguments.check:
        check_frames(arguments.out)
        return
    self_check()
    # Bake the same stride and contact timing as the painted Windows executable.
    with tempfile.TemporaryDirectory() as scratch:
        exporter = Path(scratch) / "export-poses.exe"
        subprocess.run(["rustc", "--edition=2024", "--cfg", 'feature="painted"', ROOT / "tools/export_poses.rs", "-o", exporter], check=True)
        pose_file = Path(scratch) / "poses.json"
        subprocess.run([exporter, pose_file], check=True)
        contents = pose_file.read_bytes()
    clips = json.loads(contents)
    for action, data in clips.items():
        assert len(data["poses"]) == COUNTS[action], (action, "Runtime pose count and artwork disagree")
    if arguments.clip:
        previous = json.loads((arguments.out / "poses.json").read_bytes())
        # All three walking assets use one foot-placement table.
        shared_walk = {"walk", "stalk", "settle"}
        if (arguments.clip in shared_walk and clips["walk"] != previous["walk"]) or any(
                clips[name] != previous[name] for name in clips if name != arguments.clip):
            parser.error("Other pose tables have changed; run a full bake")
    arguments.out.mkdir(parents=True, exist_ok=True)
    jobs = []
    for action, count in COUNTS.items():
        if arguments.clip and action != arguments.clip:
            continue
        directory = arguments.out / action
        directory.mkdir(exist_ok=True)
        samples = {"walk": [0, .16, .32, .5, .82, .99], "look": [0, .2, .53, .99],
                   "peck": [0, .15, .19, .25, .49, .99], "stalk": [0, .16, .32, .5, .82, .99],
                   "settle": [0, .43, 1]}
        indices = [round(f * (count - 1)) for f in samples[action]] if arguments.samples else range(count)
        jobs.extend((action, index) for index in indices)
    os.environ["OPENBLAS_NUM_THREADS"] = "1"
    with ProcessPoolExecutor(max_workers=arguments.jobs, initializer=init_worker, initargs=(clips, arguments.out)) as pool:
        for n, action in enumerate(pool.map(bake_frame, jobs, chunksize=4), 1):
            if n % 64 == 0 or n == len(jobs):
                print(f"{action}: {n}/{len(jobs)}", flush=True)
    (arguments.out / "poses.json").write_bytes(contents)
    if not arguments.samples:
        if arguments.clip in (None, "settle", "walk"):
            with Image.open(arguments.out / "settle/0000.png") as side, Image.open(ASSETS / "rig/turn-views.png") as sheet:
                bake_turn(side.convert("RGBA"), sheet.convert("RGBA"), arguments.out / "turn")
        write_validation(arguments.out)


if __name__ == "__main__":
    main()
