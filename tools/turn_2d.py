"""Register three painted views around the feet to bake the right-to-front pivot."""
import argparse
from pathlib import Path

import numpy as np
from PIL import Image

from sprite_image import resize_rgba
from cere_2d import shade_cere
from peck_plumage import _sample


ROOT = Path(__file__).resolve().parents[1]
SIZE = (512, 408)
TURN_FRAMES = 80
# Homologous painted points in side, oblique and front views, in sprite pixels.
# Each row follows one feature through the turn. Pink toes and both wing bars
# get independent points because they travel differently from the breast.
GUIDES = np.array([
    [(410,89),(329,73),(272,75)],   # crown
    [(384,101),(304,82),(252,87)],
    [(414,107),(325,96),(253,96)],  # visible iris
    [(437,102),(354,94),(291,96)],  # far temple comes into view
    [(442,112),(352,103),(272,104)],  # cere
    [(463,126),(366,122),(272,120)],  # beak
    [(434,132),(349,128),(275,132)],  # throat
    [(366,120),(293,122),(247,127)],  # nape
    [(323,145),(277,153),(221,154)],  # back of the neck
    [(263,166),(250,167),(207,177)],  # shoulder
    [(325,171),(297,166),(223,169)],
    [(327,223),(300,220),(214,220)],  # near coverts
    [(270,239),(271,239),(211,238)],  # front wing bar
    [(227,254),(234,252),(207,250)],  # rear wing bar
    [(124,265),(137,275),(196,240)],  # primaries
    [(104,296),(124,306),(202,262)],  # tail
    [(190,277),(193,289),(211,271)],
    [(239,282),(236,291),(224,284)],  # belly above the dark foot
    [(262,312),(253,310),(237,311)],
    [(290,282),(278,300),(272,301)],  # middle of belly
    [(338,257),(335,271),(316,279)],
    [(394,217),(382,204),(351,220)],  # breast silhouette
    [(409,170),(367,153),(320,160)],
    [(232,328),(242,334),(237,335)],  # left tarsus and toes
    [(214,334),(216,344),(203,344)],
    [(246,343),(248,353),(236,353)],
    [(267,339),(277,345),(263,344)],
    [(266,317),(295,311),(304,312)],  # right tarsus and toes
    [(283,332),(302,332),(308,334)],
    [(268,338),(282,338),(280,344)],
    [(295,352),(316,349),(312,353)],
    [(320,344),(346,342),(340,345)],
], dtype=float).transpose(1, 0, 2)


def round_belly(sprite, frontal):
    # Move only a strip of lower breast plumage; the feather cuffs and feet stay planted.
    y, x = np.mgrid[:SIZE[1], :SIZE[0]]
    if frontal:
        across = x - 271.5
        edge_y = 301 - .0037 * across**2 - .0000008 * across**4
        width = across / 103
    else:
        across = x - 340
        edge_y = 281 - .86 * across - .006 * across**2
        width = across / 63
    depth = (y - edge_y) / 43
    strip = np.maximum(1 - width**2, 0)**2 * np.maximum(1 - depth**2, 0)**2
    taper = np.clip((312 - y) / 10, 0, 1)
    strip *= taper**2 * (3 - 2*taper)
    if frontal:
        # The sides gain width while the centre between the legs curves down slightly.
        dx = 4 * across / 45 * strip
        dy = 2.25 * strip
    else:
        dx, dy = 1.5 * strip, 3.5 * strip
    sampled = _sample(np.asarray(sprite.convert('RGBa'), dtype=np.float32), x - dx, y - dy)
    rounded = Image.fromarray(np.rint(sampled).astype('uint8'), 'RGBa').convert('RGBA')
    return Image.composite(rounded, sprite, Image.fromarray(strip > .001))


def turn_keys(sheet):
    assert sheet.size == (1774, 887), 'The landmarks belong to the original turn painting'
    keys = []
    for left, axis, base, scale, cere in ((0, 532, 865, .345, (725,111,800,172)),
                                          (887, 474, 860, .344, (438,107,512,158))):
        paint = sheet.crop((left, 0, left+887, 887))
        shade_cere(paint, cere)
        paint = resize_rgba(paint, (round(887*scale),)*2)
        sprite = Image.new('RGBA', SIZE)
        sprite.alpha_composite(paint, (round(272-axis*scale), round(354-base*scale)))
        keys.append(round_belly(sprite, frontal=left != 0))
    # A frontal painting has just one silhouette when it crosses the mirror axis.
    front = keys[-1]
    right = front.crop((0,0,272,SIZE[1])).transpose(Image.Transpose.FLIP_LEFT_RIGHT)
    front.paste(right.crop((0,0,240,SIZE[1])), (272,0))
    return keys


def radial(points, centers):
    squared = ((points[...,None,:]-centers)**2).sum(axis=-1)
    return squared*np.log(np.maximum(squared, 1e-12))


def warp(pixels, source, target):
    # A thin-plate spline lets the neck curve while the iris, bill and toes stay registered.
    edges = [(8,8),(503,8),(8,397),(503,397)]
    a = np.vstack([target,edges])/256
    b = np.vstack([source,edges])/256
    basis = np.column_stack([np.ones(len(a)),a])
    kernel = radial(a,a) + np.eye(len(a))*1e-5
    weights = np.linalg.solve(np.block([[kernel,basis],[basis.T,np.zeros((3,3))]]),
                              np.vstack([b-a,np.zeros((3,2))]))
    yy,xx = np.mgrid[0:SIZE[1]+4:4,0:SIZE[0]+4:4]
    grid = np.stack([xx,yy],axis=-1)/256
    samples = (radial(grid,a)@weights[:-3] + weights[-3] + grid@weights[-2:] + grid)*256
    y,x = np.mgrid[:SIZE[1],:SIZE[0]]/4
    ix,iy = x.astype(int),y.astype(int)
    fx,fy = (x-ix)[...,None],(y-iy)[...,None]
    coords = ((samples[iy,ix]*(1-fx)+samples[iy,ix+1]*fx)*(1-fy)
            +(samples[iy+1,ix]*(1-fx)+samples[iy+1,ix+1]*fx)*fy)
    return _sample(pixels,coords[...,0],coords[...,1])


def signed_edge(alpha):
    # Distances interpolate a single solid silhouette instead of two translucent outlines.
    def distance(field):
        for axis in (0, 1):
            ramp = np.arange(field.shape[axis]).reshape((-1,1) if axis == 0 else (1,-1))
            forward = np.minimum.accumulate(field-ramp, axis=axis)+ramp
            backward = np.flip(np.minimum.accumulate(np.flip(field+ramp,axis), axis=axis),axis)-ramp
            field = np.minimum(forward,backward)
        return field
    coverage = alpha/255
    inside = distance(np.where(coverage<.995,coverage-.5,1024))
    outside = distance(np.where(coverage>.005,.5-coverage,1024))
    return np.where(coverage>.5,inside,-outside)


def bake(side, sheet, destination):
    keys = [side, *turn_keys(sheet)]
    textures = [np.asarray(im.convert('RGBa'),dtype=np.float32) for im in keys]
    destination.mkdir(parents=True,exist_ok=True)
    last = TURN_FRAMES - 1
    oblique = round(last * 7 / 15)
    for i in range(TURN_FRAMES):
        if i in (0, oblique, last):
            result = keys[(0, oblique, last).index(i)]
        else:
            k = 0 if i < oblique else 1
            t = i / oblique if k == 0 else (i - oblique) / (last - oblique)
            points = GUIDES[k]*(1-t)+GUIDES[k+1]*t
            # The near toes lift slightly while the other foot takes the weight.
            points[-5:,1] -= 3*np.sin(np.pi*t)
            a = warp(textures[k], GUIDES[k], points)
            b = warp(textures[k+1], GUIDES[k+1], points)
            blend = np.clip((t-.19)/.63,0,1)
            blend = blend*blend*(3-2*blend)
            blended = a*(1-blend)+b*blend
            silhouette = t*t*(3-2*t)
            alpha = np.clip(.5 + signed_edge(a[...,3])*(1-silhouette) + signed_edge(b[...,3])*silhouette,0,1)*255
            # Where only one painting supplies colour, retain that feather paint at full coverage.
            pigment = a+b
            color = np.where(blended[...,3:]>1,blended[...,:3]/np.maximum(blended[...,3:],.001),
                             pigment[...,:3]/np.maximum(pigment[...,3:],.001))
            result = Image.fromarray(np.rint(np.dstack([color*255,alpha])).clip(0,255).astype('uint8'),'RGBA')
        alpha = result.getchannel('A')
        box = alpha.point(lambda p: 255 if p>100 else 0).getbbox()
        assert box and box[0]>2 and box[2]<510 and 348<=box[3]<=357, (i, box)
        result.save(destination/f'{i:04d}.png', compress_level=6)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--side',type=Path,default=ROOT/'assets-2d/settle/0000.png')
    parser.add_argument('--sheet',type=Path,default=ROOT/'assets-2d/rig/turn-views.png')
    parser.add_argument('--out',type=Path,default=ROOT/'assets-2d/turn')
    args=parser.parse_args()
    with Image.open(args.side) as side, Image.open(args.sheet) as sheet:
        bake(side.convert('RGBA'),sheet.convert('RGBA'),args.out)
