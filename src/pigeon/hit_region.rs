use super::{DESTINATION, FRAME_HEIGHT, FRAME_PITCH, FRAME_WIDTH, PigeonRenderer, SpriteView};
use windows::{
    Win32::{
        Foundation::{E_FAIL, RECT},
        Graphics::Gdi::{ExtCreateRegion, HRGN, RDH_RECTANGLES, RGNDATAHEADER},
    },
    core::{Error, Result},
};

impl PigeonRenderer {
    pub(crate) fn body_region(
        &self,
        view: SpriteView,
        origin_x: f32,
        origin_y: f32,
    ) -> Result<HRGN> {
        debug_assert_eq!(self.last_frame, Some(view.frame));
        body_region(&self.pixels, view, origin_x, origin_y)
    }
}

struct Sample {
    first: usize,
    second: usize,
    weight: f32,
}

// Sample the same texel centers and linear filter as DrawBitmap, including when mirrored.
fn samples(start: f32, end: f32, texels: u32) -> (i32, Vec<Sample>) {
    let first_pixel = (start.min(end) - 0.5).ceil() as i32;
    let past_last = (start.max(end) - 0.5).ceil() as i32;
    let step = texels as f32 / (end - start);
    let last = texels as usize - 1;
    let axis = (first_pixel..past_last)
        .map(|pixel| {
            let t = ((pixel as f32 + 0.5 - start) * step - 0.5).clamp(0.0, last as f32);
            let first = t as usize;
            Sample {
                first,
                second: (first + 1).min(last),
                weight: t - first as f32,
            }
        })
        .collect();
    (first_pixel, axis)
}

fn body_region(pixels: &[u8], view: SpriteView, origin_x: f32, origin_y: f32) -> Result<HRGN> {
    let scale_x = view.scale * view.facing;
    let (left, columns) = samples(
        origin_x + DESTINATION.left * scale_x,
        origin_x + DESTINATION.right * scale_x,
        FRAME_WIDTH,
    );
    let (top, rows) = samples(
        origin_y + DESTINATION.top * view.scale,
        origin_y + DESTINATION.bottom * view.scale,
        FRAME_HEIGHT,
    );
    // RGNDATA is an aligned header immediately followed by a variable number of RECTs.
    const HEADER: usize = size_of::<RGNDATAHEADER>() / size_of::<RECT>();
    const {
        assert!(
            HEADER * size_of::<RECT>() == size_of::<RGNDATAHEADER>()
                && align_of::<RECT>() >= align_of::<RGNDATAHEADER>()
        )
    };
    let mut data = vec![RECT::default(); HEADER];
    let mut bounds = RECT {
        left: i32::MAX,
        top: i32::MAX,
        right: i32::MIN,
        bottom: i32::MIN,
    };
    for (dy, row) in rows.iter().enumerate() {
        let y = top + dy as i32;
        let row_bytes = FRAME_PITCH as usize;
        let upper = &pixels[row.first * row_bytes..(row.first + 1) * row_bytes];
        let lower = &pixels[row.second * row_bytes..(row.second + 1) * row_bytes];
        let covered = columns.iter().map(|col| {
            let a = col.first * 4 + 3;
            let b = col.second * 4 + 3;
            let hi = upper[a] as f32 + col.weight * (upper[b] as f32 - upper[a] as f32);
            let lo = lower[a] as f32 + col.weight * (lower[b] as f32 - lower[a] as f32);
            hi + row.weight * (lo - hi) >= 24.0
        });
        let mut start = None;
        for (dx, opaque) in covered.chain(Some(false)).enumerate() {
            let x = left + dx as i32;
            if opaque {
                start.get_or_insert(x);
            } else if let Some(left) = start.take() {
                data.push(RECT {
                    left,
                    top: y,
                    right: x,
                    bottom: y + 1,
                });
                bounds.left = bounds.left.min(left);
                bounds.right = bounds.right.max(x);
                bounds.top = bounds.top.min(y);
                bounds.bottom = y + 1;
            }
        }
    }
    let count = data.len() - HEADER;
    let header = RGNDATAHEADER {
        dwSize: size_of::<RGNDATAHEADER>() as u32,
        iType: RDH_RECTANGLES,
        nCount: count as u32,
        nRgnSize: (count * size_of::<RECT>()) as u32,
        rcBound: if count == 0 { RECT::default() } else { bounds },
    };
    unsafe {
        data.as_mut_ptr().cast::<RGNDATAHEADER>().write(header);
        let region = ExtCreateRegion(
            None,
            size_of_val(data.as_slice()) as u32,
            data.as_ptr().cast(),
        );
        if region.is_invalid() {
            Err(Error::new(E_FAIL, "Could not create pigeon body region"))
        } else {
            Ok(region)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pigeon::FRAME_BYTES;
    use windows::Win32::Graphics::Gdi::{DeleteObject, PtInRegion};

    #[test]
    fn visible_details_and_clear_gaps_follow_facing_and_scale() {
        let mut pixels = vec![0; FRAME_BYTES];
        for y in 40..52 {
            for x in 60..100 {
                pixels[(y * FRAME_WIDTH as usize + x) * 4 + 3] = match x {
                    70..80 => 0,
                    90..100 => 16,
                    _ => 255,
                };
            }
        }
        for (scale, facing) in [
            (0.575, 1.0),
            (0.84, -1.0),
            (1.0, 1.0),
            (1.15, -1.0),
            (2.0, 1.0),
        ] {
            let view = SpriteView::new(("walk", 0.0), facing, scale);
            let (ox, oy) = (160.0 * scale, 145.0 * scale);
            let region = body_region(&pixels, view, ox, oy).unwrap();
            for (x, y, hit) in [
                (65., 46., true),
                (75., 46., false),
                (85., 46., true),
                (95., 46., false),
                (65., 58., false),
            ] {
                let cx = (ox + (DESTINATION.left + x / 2.0) * facing * scale).floor() as i32;
                let cy = (oy + (DESTINATION.top + y / 2.0) * scale).floor() as i32;
                assert_eq!(unsafe { PtInRegion(region, cx, cy) }.as_bool(), hit);
            }
            assert!(unsafe { DeleteObject(region.into()) }.as_bool());
        }
    }
}
