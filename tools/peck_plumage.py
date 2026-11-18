"""Register the two paintings before blending the folded wing and arched back."""
import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter


KEY_SCALE = 0.59
KEY_BILL = np.array((1326., 947.))
CONTACT = np.array((458., 353.))


def key_point(point):
    return (np.asarray(point) - KEY_BILL) * KEY_SCALE + CONTACT


def prepare_key(original):
    # Remove the two legs and their feather cuffs. The moving leg rig supplies these.
    box = (791, 593, 1365, 963)
    key = original.crop(box)
    mask = Image.new('L', key.size)
    outline = [(789,590),(1368,590),(1368,964),(1230,964),(1187,872),
               (1143,870),(1095,879),(1069,881),(1030,867),
               (997,853),(963,839),(934,819),(906,798),(789,792)]
    ImageDraw.Draw(mask).polygon([(x-box[0],y-box[1]) for x,y in outline], fill=255)
    key.putalpha(ImageChops.multiply(key.getchannel('A').filter(ImageFilter.MinFilter(3)), mask.filter(ImageFilter.GaussianBlur(1))))
    return key


def _profile(pixels):
    alpha = pixels[...,3] > 24
    occupied = np.flatnonzero(np.any(alpha,axis=0))
    # Transparent margin columns repeat the nearest valid section.
    xs = np.arange(alpha.shape[1])
    def guide(edges):
        profile = np.interp(xs, occupied, edges[occupied])
        # Tail tips come in separate vanes. Following each individual notch pinches the
        # outer feather during a bow; extrapolate its shaft instead.
        width = occupied[-1] - occupied[0]
        start, stop = occupied[0] + round(0.09*width), occupied[0] + round(0.17*width)
        slope = np.polyfit(xs[start:stop], profile[start:stop], 1)[0]
        profile[:start] = profile[start] + (xs[:start]-start)*slope
        kernel_x = np.arange(-6,7)
        kernel = np.exp(-0.5*(kernel_x/2.6)**2)
        return np.convolve(np.pad(profile,6,mode='edge'), kernel/kernel.sum(), mode='valid')
    top = guide(np.argmax(alpha,axis=0))
    bottom = guide((len(alpha)-1)-np.argmax(alpha[::-1],axis=0))
    return occupied[[0,-1]], top, np.maximum(bottom,top+1)


def _sample(pixels, x, y):
    x = np.clip(x,0,pixels.shape[1]-1.001)
    y = np.clip(y,0,pixels.shape[0]-1.001)
    ix,iy=x.astype(int),y.astype(int)
    fx,fy=(x-ix)[...,None],(y-iy)[...,None]
    return ((pixels[iy,ix]*(1-fx)+pixels[iy,ix+1]*fx)*(1-fy)
            +(pixels[iy+1,ix]*(1-fx)+pixels[iy+1,ix+1]*fx)*fy)


def key_weight(peck):
    t = float(np.clip((peck-0.60)/0.38, 0, 1))
    return t * (t * (3 - 2*t))


def painted_peck(plumage, key, peck, closure, landmarks, supersample):
    # The key is introduced during the fast bow; its contours align before its colours change.
    t = key_weight(peck)
    if not t:
        return plumage
    key = key.copy()
    if closure > 0:
        # The peck painting sees the eye from a different angle. Match its own orbital ring.
        eye_box = (500, 253, 536, 289)
        px = np.array(key.crop(eye_box))
        y,x = np.mgrid[-18:18,-16:20]
        width = 13.7*np.sqrt(np.maximum(0,1-(x/13.5)**2))
        middle = 0.3*x
        cap=np.clip(width-np.abs(y-middle)+0.5,0,1)*np.clip(np.abs(y-middle)-width*(1-closure)+0.5+closure**6,0,1)
        cap*=np.clip(14-np.abs(x),0,1)
        lid=np.asarray(key.crop((462,253,498,289)))[...,:3].astype(float)
        lid*=1-0.45*closure*np.exp(-((y-middle+width*(1-closure))/0.9)**2)[...,None]
        px[...,:3]=np.rint(px[...,:3]*(1-cap[...,None])+lid*cap[...,None])
        key.paste(Image.fromarray(px), eye_box[:2])
    old = np.asarray(plumage.convert('RGBa'),dtype=np.float32)
    new = np.asarray(key.convert('RGBa'),dtype=np.float32)
    old_limits,old_top,old_bottom = _profile(old)
    new_limits,new_top,new_bottom = _profile(new)
    # Corresponding landmarks: tail, the two wing bars, forward wing coverts, and eye.
    old_marks=np.r_[old_limits[0], supersample * landmarks, old_limits[1]]
    new_marks=np.array([new_limits[0],900-791,974-791,1016-791,1121-791,1307-791,new_limits[1]])
    assert np.all(np.diff(old_marks)>0) and np.all(np.diff(new_marks)>0), 'Wing landmarks must remain in anatomical order'
    k = KEY_SCALE*supersample
    offset = key_point((791,593))*supersample
    target_marks=old_marks*(1-t)+(new_marks*k+offset[0])*t
    # Pad at both ends to retain the antialiased outermost pixels.
    left=max(0,int(target_marks[0])-6);right=min(old.shape[1],int(target_marks[-1])+8)
    xs=np.arange(left,right)
    old_x=np.interp(xs,np.r_[target_marks[0]-8,target_marks,target_marks[-1]+8],
                    np.r_[old_marks[0]-8,old_marks,old_marks[-1]+8])
    new_x=np.interp(xs,np.r_[target_marks[0]-8,target_marks,target_marks[-1]+8],
                    np.r_[new_marks[0]-8/k,new_marks,new_marks[-1]+8/k])
    def at_x(profile,x):
        return np.interp(x,np.arange(profile.size),profile)
    ot,ob=at_x(old_top,old_x),at_x(old_bottom,old_x)
    nt,nb=at_x(new_top,new_x),at_x(new_bottom,new_x)
    top=ot*(1-t)+(offset[1]+k*nt)*t
    bottom=ob*(1-t)+(offset[1]+k*nb)*t
    y0=max(0,int(top.min())-50);y1=min(old.shape[0],int(bottom.max())+50)
    height=(np.arange(y0,y1)[:,None]-top)/np.maximum(bottom-top,1)
    a=_sample(old,old_x[None,:],ot+height*(ob-ot))
    b=_sample(new,new_x[None,:],nt+height*(nb-nt))
    # Let the aligned texture change at mid-bow rather than exposing an intermediate double wing.
    material=float(np.clip((t-0.28)/0.43,0,1))
    material *= material*(3-2*material)
    out=np.zeros(old.shape,dtype=np.uint8)
    out[y0:y1,left:right]=np.rint(a*(1-material)+b*material)
    return Image.fromarray(out,'RGBa').convert('RGBA')
