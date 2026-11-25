use super::*;

pub(super) struct WrapWindow {
    pub hwnd: HWND,
    pub body: HWND,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

impl WrapWindow {
    pub unsafe fn new(app: &App) -> Result<Self> {
        unsafe {
            let width = (WIDTH * app.base_scale) as i32;
            let height = (HEIGHT * app.base_scale) as i32;
            let module = GetModuleHandleW(None)?;
            let create = |style, owner| {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | style,
                    BODY_CLASS,
                    PCWSTR::null(),
                    WS_POPUP,
                    0,
                    0,
                    width,
                    height,
                    Some(owner),
                    None,
                    Some(module.into()),
                    None,
                )
            };
            let hwnd = create(WS_EX_LAYERED | WS_EX_TRANSPARENT, app.hwnd)?;
            let result = (|| {
                let (target, visual) =
                    Graphics::attach(&app.graphics.composition, &app.graphics.swap, hwnd)?;
                let body = create(WS_EX_NOREDIRECTIONBITMAP, app.hwnd)?;
                SetWindowLongPtrW(body, GWLP_USERDATA, 1);
                Ok(Self {
                    hwnd,
                    body,
                    _target: target,
                    _visual: visual,
                })
            })();
            if result.is_err() {
                let _ = DestroyWindow(hwnd);
            }
            result
        }
    }
}

pub(super) fn other_x(anchor: f32, reach: f32, row: (i32, i32), x: i32) -> Option<i32> {
    let (left, right) = row;
    if anchor - reach < left as f32 {
        Some(x + (right - left))
    } else if anchor + reach > right as f32 {
        Some(x - (right - left))
    } else {
        None
    }
}

pub(super) fn clip_x(x: i32, width: i32, row: (i32, i32)) -> Option<(i32, i32)> {
    let clip = ((row.0 - x).clamp(0, width), (row.1 - x).clamp(0, width));
    (clip != (0, width)).then_some(clip)
}

pub(super) struct Region(pub HRGN);

impl Drop for Region {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.0.into());
            }
        }
    }
}

impl Region {
    pub fn rectangle(left: i32, right: i32, height: i32) -> Result<Self> {
        let region = unsafe { CreateRectRgn(left, 0, right, height) };
        if region.is_invalid() {
            return Err(windows::core::Error::from_thread());
        }
        Ok(Self(region))
    }

    pub fn copy(&self) -> Result<Self> {
        let result = Self::rectangle(0, 0, 0)?;
        if unsafe { CombineRgn(Some(result.0), Some(self.0), None, RGN_COPY) } == RGN_ERROR {
            return Err(windows::core::Error::from_thread());
        }
        Ok(result)
    }

    pub fn intersect(&self, (left, right): (i32, i32), height: i32) -> Result<()> {
        let clip = Self::rectangle(left, right, height)?;
        if unsafe { CombineRgn(Some(self.0), Some(self.0), Some(clip.0), RGN_AND) } == RGN_ERROR {
            return Err(windows::core::Error::from_thread());
        }
        Ok(())
    }

    pub fn set(mut self, hwnd: HWND) -> Result<()> {
        if unsafe { SetWindowRgn(hwnd, Some(self.0), false) } == 0 {
            return Err(windows::core::Error::new(
                E_FAIL,
                text(
                    "Could not set the pigeon's window region",
                    "无法设置鸽子的窗口区域",
                ),
            ));
        }
        self.0 = HRGN::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_positions_are_one_row_apart_and_clips_are_complementary() {
        let row = (-1920, 2560);
        let reach = 150.0;
        let width = 400;
        for (anchor, x) in [(-1910.25, -2110), (2550.75, 2351)] {
            let other = other_x(anchor, reach, row, x).unwrap();
            assert_eq!((other - x).abs(), row.1 - row.0);
            let a = clip_x(x, width, row).unwrap();
            let b = clip_x(other, width, row).unwrap();
            assert!(
                a.1 == b.0 && a.0 == 0 && b.1 == width || b.1 == a.0 && b.0 == 0 && a.1 == width
            );
        }
        assert_eq!(other_x(320.0, reach, row, 120), None);
    }
}
