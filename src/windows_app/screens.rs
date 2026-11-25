use super::*;
use windows::core::BOOL;

// Includes a pixel of clearance around all baked poses, even looking up and turning.
const HEAD_CLEARANCE: f32 = 155.0;
const FEET_CLEARANCE: f32 = 5.0;

#[derive(Clone, Copy)]
pub(super) struct Screen {
    pub monitor: HMONITOR,
    pub rect: RECT,
    pub work: RECT,
}

impl Screen {
    pub fn visible_at(self, feet_y: f32, scale: f32) -> bool {
        let (top, bottom) = self.vertical(scale);
        (top..=bottom).contains(&feet_y)
    }

    pub fn vertical(self, scale: f32) -> (f32, f32) {
        let work_top = self.work.top as f32;
        let bottom = (self.work.bottom as f32 - FEET_CLEARANCE * scale).max(work_top);
        ((work_top + HEAD_CLEARANCE * scale).min(bottom), bottom)
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
            let rect = *rect;
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let work = if GetMonitorInfoW(monitor, &mut info).as_bool() {
                info.rcWork
            } else {
                rect
            };
            (*(data.0 as *mut Vec<Screen>)).push(Screen {
                monitor,
                rect,
                work,
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

pub(super) fn row(screens: &[Screen], screen: Screen, y: f32, scale: f32) -> (i32, i32) {
    let mut edges = (screen.work.left, screen.work.right);
    loop {
        let previous = edges;
        for s in screens.iter().filter(|s| s.visible_at(y, scale)) {
            // Only traverse screens sharing this row without crossing an empty desktop area.
            if s.work.left <= edges.1 && s.work.right >= edges.0 {
                edges = (edges.0.min(s.work.left), edges.1.max(s.work.right));
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
    fn head_and_feet_fit_between_taskbars_at_each_size() {
        let rect = RECT {
            left: 0,
            top: -240,
            right: 2560,
            bottom: 1360,
        };
        let stacked_bars = Screen {
            monitor: HMONITOR::default(),
            rect,
            work: RECT {
                top: -180,
                bottom: 1230,
                ..rect
            },
        };
        for scale in [0.8625, 1.725] {
            let (top, bottom) = stacked_bars.vertical(scale);
            assert!(top - 153.5 * scale >= stacked_bars.work.top as f32);
            assert!(bottom + 4.0 * scale <= stacked_bars.work.bottom as f32);
        }
        let bare = Screen {
            work: rect,
            ..stacked_bars
        };
        assert_eq!(bare.vertical(1.0), (-85.0, 1355.0));
    }

    #[test]
    fn a_side_taskbar_is_excluded_from_the_horizontal_wrap() {
        let rect = RECT {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1600,
        };
        let screen = Screen {
            monitor: HMONITOR::default(),
            rect,
            work: RECT { left: 56, ..rect },
        };
        assert_eq!(row(&[screen], screen, 800.0, 1.0), (56, 2560));
    }

    #[test]
    fn row_connects_adjacent_screens_but_excludes_gaps_and_stacked_screens() {
        let screen = |left, top, right, bottom| {
            let rect = RECT {
                left,
                top,
                right,
                bottom,
            };
            Screen {
                monitor: HMONITOR::default(),
                rect,
                work: rect,
            }
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
        assert_eq!(row(&layout, main, 500.0, 1.0), (-3200, 2560));
        assert_eq!(row(&layout, adjacent, 1200.0, 1.0), (-1920, 2560));
        assert_eq!(row(&layout, main, 300.0, 1.0), (0, 2560));
        assert_eq!(row(&layout, main, 1320.0, 1.0), (0, 2560));
    }
}
