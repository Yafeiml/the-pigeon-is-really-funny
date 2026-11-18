use super::{Event, SIZE_MAX, SIZE_MIN, enqueue};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{Error, HSTRING, Result, w},
};

const CLASS: windows::core::PCWSTR = w!("Pigeon.Native.SizeSlider");
const TBM_GETPOS: u32 = WM_USER;

fn title(percent: u32) -> HSTRING {
    format!("鸽子大小 · {percent}%").into()
}

unsafe extern "system" fn slider_keys(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_KEYDOWN && wp.0 == VK_ESCAPE.0 as usize {
            if let Ok(panel) = GetParent(hwnd) {
                let _ = PostMessageW(Some(panel), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            return LRESULT(0);
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(slider_keys), id);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

unsafe extern "system" fn panel_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_HSCROLL if lp.0 != 0 => {
                // TBM_GETPOS includes live thumb tracking and keyboard changes.
                let percent = SendMessageW(HWND(lp.0 as _), TBM_GETPOS, None, None).0 as u32;
                if (SIZE_MIN..=SIZE_MAX).contains(&percent) {
                    let _ = SetWindowTextW(hwnd, &title(percent));
                    enqueue(Event::Size(percent));
                }
                LRESULT(0)
            }
            WM_ACTIVATE if wp.0 as u16 == WA_INACTIVE as u16 => {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn create(owner: HWND, anchor: POINT, percent: u32) -> Result<(HWND, HWND)> {
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        })
        .ok()?;
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(panel_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(Error::from_thread());
        }

        let monitor = MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info).ok()?;
        let (mut dpi, mut dpi_y) = (0, 0);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y)?;
        let px = |n: i32| (n * dpi as i32 + 48) / 96;
        let (width, height) = (px(220), px(42));
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let ex_style = WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
        AdjustWindowRectExForDpi(&mut rect, style, false, ex_style, dpi)?;
        let (outer_w, outer_h) = (rect.right - rect.left, rect.bottom - rect.top);
        let work = info.rcWork;
        let mut x = anchor
            .x
            .clamp(work.left, (work.right - outer_w).max(work.left));
        let mut y = anchor
            .y
            .clamp(work.top, (work.bottom - outer_h).max(work.top));
        let mut bird = RECT::default();
        if IsWindowVisible(owner).as_bool()
            && GetWindowRect(owner, &mut bird).is_ok()
            && x < bird.right
            && x + outer_w > bird.left
            && y < bird.bottom
            && y + outer_h > bird.top
        {
            // Keep the live preview in view when the slider is opened from the bird.
            if let Some(pos) = [
                (bird.left - outer_w, y),
                (bird.right, y),
                (x, bird.top - outer_h),
                (x, bird.bottom),
            ]
            .into_iter()
            .filter(|&(cx, cy)| {
                cx >= work.left
                    && cy >= work.top
                    && cx + outer_w <= work.right
                    && cy + outer_h <= work.bottom
            })
            .min_by_key(|&(cx, cy)| cx.abs_diff(x) + cy.abs_diff(y))
            {
                (x, y) = pos;
            }
        }
        let percent = percent.clamp(SIZE_MIN, SIZE_MAX);
        let panel = CreateWindowExW(
            ex_style,
            CLASS,
            &title(percent),
            style,
            x,
            y,
            outer_w,
            outer_h,
            Some(owner),
            None,
            Some(instance),
            None,
        )?;
        let slider = (|| -> Result<HWND> {
            let slider = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                TRACKBAR_CLASSW,
                w!("鸽子大小"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(TBS_AUTOTICKS),
                px(8),
                px(5),
                width - px(16),
                height - px(8),
                Some(panel),
                None,
                Some(instance),
                None,
            )?;
            let range = LPARAM(((SIZE_MAX << 16) | SIZE_MIN) as isize);
            SendMessageW(slider, TBM_SETRANGE, Some(WPARAM(1)), Some(range));
            SendMessageW(slider, TBM_SETTICFREQ, Some(WPARAM(10)), None);
            SendMessageW(
                slider,
                TBM_SETPOS,
                Some(WPARAM(1)),
                Some(LPARAM(percent as isize)),
            );
            SetWindowSubclass(slider, Some(slider_keys), 1, 0).ok()?;
            Ok(slider)
        })();
        match slider {
            Ok(slider) => Ok((panel, slider)),
            Err(e) => {
                let _ = DestroyWindow(panel);
                Err(e)
            }
        }
    }
}

pub(super) fn show(owner: HWND, anchor: POINT, percent: u32) -> Result<()> {
    let (panel, slider) = create(owner, anchor, percent)?;
    unsafe {
        let _ = ShowWindow(panel, SW_SHOW);
        let _ = SetForegroundWindow(panel);
        let _ = SetFocus(Some(slider));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_slider_updates_caption_and_size_one_percent_at_a_time() {
        let (panel, slider) = create(HWND::default(), POINT::default(), 73).unwrap();
        unsafe {
            assert_eq!(SendMessageW(slider, TBM_GETPOS, None, None).0, 73);
            for (key, expected) in [
                (VK_HOME, SIZE_MIN),
                (VK_RIGHT, SIZE_MIN + 1),
                (VK_END, SIZE_MAX),
            ] {
                let key = Some(WPARAM(key.0 as usize));
                SendMessageW(slider, WM_KEYDOWN, key, None);
                SendMessageW(slider, WM_KEYUP, key, None);
                assert_eq!(
                    SendMessageW(slider, TBM_GETPOS, None, None).0,
                    expected as isize
                );
                let mut text = [0; 32];
                let len = GetWindowTextW(panel, &mut text);
                assert_eq!(
                    String::from_utf16_lossy(&text[..len as usize]),
                    title(expected).to_string()
                );
                super::super::EVENTS.with(|events| {
                    assert!(
                        matches!(events.borrow_mut().pop(), Some(Event::Size(p)) if p == expected)
                    );
                });
            }
            SendMessageW(slider, WM_KEYDOWN, Some(WPARAM(VK_ESCAPE.0 as usize)), None);
            let mut close = MSG::default();
            assert!(PeekMessageW(&mut close, Some(panel), WM_CLOSE, WM_CLOSE, PM_REMOVE).as_bool());
            DispatchMessageW(&close);
            assert!(!IsWindow(Some(panel)).as_bool());
            assert!(!PeekMessageW(&mut close, None, WM_QUIT, WM_QUIT, PM_REMOVE).as_bool());
        }
    }
}
