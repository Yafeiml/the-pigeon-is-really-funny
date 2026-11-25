use super::*;
use windows::core::BOOL;

#[derive(Clone, Copy)]
pub(super) struct Screen {
    pub monitor: HMONITOR,
    pub rect: RECT,
}

impl Screen {
    pub fn visible_at(self, feet_y: f32, top_margin: f32) -> bool {
        feet_y >= self.top(top_margin) && feet_y < self.rect.bottom as f32
    }

    pub fn top(self, top_margin: f32) -> f32 {
        (self.rect.top as f32 + top_margin).min((self.rect.bottom - 1) as f32)
    }

    pub fn refresh_rate(self) -> Option<u32> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        let mut mode = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        unsafe {
            if !GetMonitorInfoW(self.monitor, &mut info.monitorInfo).as_bool()
                || !EnumDisplaySettingsW(
                    PCWSTR(info.szDevice.as_ptr()),
                    ENUM_CURRENT_SETTINGS,
                    &mut mode,
                )
                .as_bool()
            {
                return None;
            }
        }
        (mode.dmDisplayFrequency > 1).then_some(mode.dmDisplayFrequency)
    }
}

pub(super) fn connected() -> Result<Vec<Screen>> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        unsafe {
            (*(data.0 as *mut Vec<Screen>)).push(Screen {
                monitor,
                rect: *rect,
            })
        };
        true.into()
    }
    let mut screens = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM((&raw mut screens) as isize),
        )
        .ok()?;
    }
    if screens.is_empty() {
        return Err(windows::core::Error::new(E_FAIL, "No active display"));
    }
    Ok(screens)
}

pub(super) fn nearest(screens: &[Screen], point: POINT) -> Screen {
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    screens
        .iter()
        .copied()
        .find(|s| s.monitor == monitor)
        .unwrap_or(screens[0])
}

pub(super) fn row(screens: &[Screen], screen: Screen, y: f32, top_margin: f32) -> (i32, i32) {
    let mut edges = (screen.rect.left, screen.rect.right);
    loop {
        let previous = edges;
        for s in screens.iter().filter(|s| s.visible_at(y, top_margin)) {
            // Only traverse screens sharing this row without crossing an empty desktop area.
            if s.rect.left <= edges.1 && s.rect.right >= edges.0 {
                edges = (edges.0.min(s.rect.left), edges.1.max(s.rect.right));
            }
        }
        if edges == previous {
            return edges;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_connects_adjacent_screens_but_excludes_gaps_and_stacked_screens() {
        let screen = |left, top, right, bottom| Screen {
            monitor: HMONITOR::default(),
            rect: RECT {
                left,
                top,
                right,
                bottom,
            },
        };
        let main = screen(0, 0, 2560, 1600);
        let adjacent = screen(-1920, 240, 0, 1320);
        let layout = [
            adjacent,
            screen(0, -1080, 1920, 0),
            screen(2600, 0, 4520, 1080),
            screen(-3200, 0, -1920, 1024),
            main,
        ];
        assert_eq!(row(&layout, main, 500.0, 72.0), (-3200, 2560));
        assert_eq!(row(&layout, adjacent, 1200.0, 72.0), (-1920, 2560));
        assert_eq!(row(&layout, main, 300.0, 72.0), (0, 2560));
        assert_eq!(row(&layout, main, 1320.0, 72.0), (0, 2560));
    }
}
