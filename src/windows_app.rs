use crate::{
    motion::{Mode, Motion},
    pigeon::{GROUND_Y, PigeonRenderer, SpriteView},
};
use std::{
    cell::RefCell,
    mem::size_of,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::Common::*, Direct2D::*, Direct3D::*, Direct3D11::*, DirectComposition::*,
            Dxgi::Common::*, Dxgi::*, Gdi::*,
        },
        System::{Com::*, LibraryLoader::GetModuleHandleW, Threading::*},
        UI::{HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{Interface, PCWSTR, Result, w},
};

mod settings;
mod size_slider;
use settings::Settings;

const CLASS: PCWSTR = w!("Pigeon.Native.Prototype");
const BODY_CLASS: PCWSTR = w!("Pigeon.Native.Body");
const TITLE: PCWSTR = w!("鸽子 · The pigeon is really funny");
const TRAY_MESSAGE: u32 = WM_APP + 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
const WIDTH: f32 = 320.0;
const HEIGHT: f32 = 270.0;
const SIZE_MIN: u32 = 50;
const SIZE_MAX: u32 = 100;

enum Event {
    Menu,
    Size(u32),
    Down(POINT),
    Move(POINT),
    Up,
    Display,
    Close,
}
thread_local! { static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) }; }
fn enqueue(event: Event) {
    EVENTS.with(|q| {
        let mut q = q.borrow_mut();
        if let (Event::Size(percent), Some(Event::Size(last))) = (&event, q.last_mut()) {
            *last = *percent;
        } else {
            q.push(event);
        }
    });
}

unsafe extern "system" fn body_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_RBUTTONUP => enqueue(Event::Menu),
            WM_LBUTTONDOWN => {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                SetCapture(hwnd);
                enqueue(Event::Down(pt));
            }
            WM_MOUSEMOVE if wp.0 & 1 != 0 => {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                enqueue(Event::Move(pt));
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                enqueue(Event::Up);
            }
            WM_CAPTURECHANGED => enqueue(Event::Up),
            WM_CANCELMODE => {
                let _ = ReleaseCapture();
                enqueue(Event::Up);
            }
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        LRESULT(0)
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let _ = ValidateRect(Some(hwnd), None);
                return LRESULT(0);
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => enqueue(Event::Display),
            WM_CLOSE => enqueue(Event::Close),
            WM_DESTROY => {
                PostQuitMessage(0);
                return LRESULT(0);
            }
            TRAY_MESSAGE => match lp.0 as u32 & 0xffff {
                WM_RBUTTONUP | WM_CONTEXTMENU | NIN_SELECT | NIN_KEYSELECT => enqueue(Event::Menu),
                _ => {}
            },
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        LRESULT(0)
    }
}

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Graphics {
    context: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    _composition: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    _bitmap: ID2D1Bitmap1,
    pigeon: PigeonRenderer,
    last_presented: Option<(SpriteView, (f32, f32))>,
}

enum Presentation {
    Unchanged,
    Submitted,
    Unavailable,
}

impl Graphics {
    unsafe fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        unsafe {
            let mut device = None;
            let create = |driver, output: &mut Option<ID3D11Device>| {
                D3D11CreateDevice(
                    None,
                    driver,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(output),
                    None,
                    None,
                )
            };
            if create(D3D_DRIVER_TYPE_HARDWARE, &mut device).is_err() {
                create(D3D_DRIVER_TYPE_WARP, &mut device)?;
            }
            let device = device
                .ok_or_else(|| windows::core::Error::new(E_FAIL, "D3D11 returned no device"))?;
            let dxgi: IDXGIDevice = device.cast()?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let context = d2d_factory
                .CreateDevice(&dxgi)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForComposition(&device, &desc, None)?;
            let surface: IDXGISurface = swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            let bitmap = context.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            context.SetTarget(&bitmap);
            context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = composition.CreateTargetForHwnd(hwnd, true)?;
            let visual = composition.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            composition.Commit()?;
            let pigeon = PigeonRenderer::new(&context)?;
            Ok(Self {
                context,
                swap,
                _composition: composition,
                _target: target,
                _visual: visual,
                _bitmap: bitmap,
                pigeon,
                last_presented: None,
            })
        }
    }

    unsafe fn render(
        &mut self,
        motion: &Motion,
        scale: f32,
        origin: (f32, f32),
        body: HWND,
    ) -> Result<Presentation> {
        unsafe {
            let view = SpriteView::new(motion.sprite_clip(), motion.facing(), scale);
            if self.last_presented == Some((view, origin)) {
                return Ok(Presentation::Unchanged);
            }
            self.context.BeginDraw();
            self.context.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            let (x, y) = origin;
            let result = self.pigeon.draw(&self.context, view, x, y);
            let end = self.context.EndDraw(None, None);
            result?;
            end?;
            let status = self.swap.Present(0, DXGI_PRESENT(0));
            status.ok()?;
            if status != S_OK {
                self.last_presented = None;
                return Ok(Presentation::Unavailable);
            }
            let region = self.pigeon.body_region(view, x, y)?;
            if SetWindowRgn(body, Some(region), false) == 0 {
                // Ownership transfers to Windows only on success.
                let _ = DeleteObject(region.into());
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "Cannot set pigeon input region",
                ));
            }
            self.last_presented = Some((view, origin));
            Ok(Presentation::Submitted)
        }
    }
}

struct Tray {
    data: NOTIFYICONDATAW,
}
impl Tray {
    unsafe fn new(hwnd: HWND) -> Result<Self> {
        unsafe {
            #[repr(align(4))]
            struct IconBytes([u8; include_bytes!("tray.png").len()]);
            static ICON: IconBytes = IconBytes(*include_bytes!("tray.png"));
            let icon = CreateIconFromResourceEx(
                &ICON.0,
                true,
                0x30000,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            )?;
            let mut tray = Self {
                data: NOTIFYICONDATAW {
                    cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    uID: 1,
                    uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
                    uCallbackMessage: TRAY_MESSAGE,
                    hIcon: icon,
                    ..Default::default()
                },
            };
            let tip: Vec<u16> = "鸽子 · 右键调整大小、速度和活动模式"
                .encode_utf16()
                .collect();
            tray.data.szTip[..tip.len()].copy_from_slice(&tip);
            Shell_NotifyIconW(NIM_ADD, &tray.data).ok()?;
            tray.data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &tray.data).ok()?;
            Ok(tray)
        }
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
            let _ = DestroyIcon(self.data.hIcon);
        }
    }
}

struct App {
    hwnd: HWND,
    body: HWND,
    graphics: Graphics,
    _tray: Tray,
    motion: Motion,
    base_scale: f32,
    settings: Settings,
    monitor_hz: Option<u32>,
    work: RECT,
    hidden: bool,
    drag: Option<(POINT, f32, f32)>,
    last_position: Option<(i32, i32)>,
    offset: (f32, f32),
}
impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl App {
    fn scale(&self) -> f32 {
        self.base_scale * (self.settings.size_percent as f32 / 100.0)
    }
    fn bound_width(&self) -> f32 {
        (((self.work.right - self.work.left) as f32 - WIDTH * self.base_scale) / self.scale())
            .max(0.0)
    }
    fn bound_height(&self) -> f32 {
        (((self.work.bottom - self.work.top) as f32 - HEIGHT * self.base_scale) / self.scale())
            .max(0.0)
    }
    fn center(&mut self) {
        self.motion.set_x(self.bound_width() * 0.5);
        let height = (self.work.bottom - self.work.top) as f32;
        self.motion.set_ground(
            (height * (2.0 / 3.0) - HEIGHT * self.base_scale * 0.5) / self.scale(),
            self.bound_height(),
        );
    }
    unsafe fn set_size(&mut self, percent: u32) -> Result<()> {
        let percent = percent.clamp(SIZE_MIN, SIZE_MAX);
        if percent == self.settings.size_percent {
            return Ok(());
        }
        let old_scale = self.scale();
        self.settings.size_percent = percent;
        // Motion distances are in bird units, so the feet travel one scaled stride.
        let ratio = old_scale / self.scale();
        self.motion
            .set_x((self.motion.x() * ratio).clamp(0.0, self.bound_width()));
        self.motion
            .set_ground(self.motion.ground() * ratio, self.bound_height());
        unsafe {
            self.position()?;
            if !self.hidden && !self.settings.paused {
                self.render()?;
            }
        }
        Ok(())
    }
    unsafe fn position(&mut self) -> Result<()> {
        let (left, dx) = split_pixel(self.motion.x() * self.scale());
        let (top, dy) = split_pixel(self.motion.ground() * self.scale());
        let (x, y) = (self.work.left + left, self.work.top + top);
        self.offset = (dx, dy);
        unsafe {
            if self.last_position != Some((x, y)) {
                let mut moves = BeginDeferWindowPos(2)?;
                for hwnd in [self.hwnd, self.body] {
                    moves = DeferWindowPos(
                        moves,
                        hwnd,
                        None,
                        x,
                        y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                    )?;
                }
                EndDeferWindowPos(moves)?;
                self.last_position = Some((x, y));
            }
            if self.settings.paused && !self.hidden {
                self.render()?;
            }
            Ok(())
        }
    }
    unsafe fn show_body(&self) {
        unsafe {
            let visible = !self.hidden
                && !self.settings.click_through
                && self.graphics.last_presented.is_some();
            let _ = ShowWindow(self.body, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }
    unsafe fn render(&mut self) -> Result<Presentation> {
        unsafe {
            let first_frame = self.graphics.last_presented.is_none();
            let scale = self.scale();
            let origin = sprite_origin(self.base_scale, scale, self.offset);
            let result = self
                .graphics
                .render(&self.motion, scale, origin, self.body)?;
            if first_frame && matches!(result, Presentation::Submitted) {
                self.show_body();
            }
            Ok(result)
        }
    }
    unsafe fn popup(&mut self) -> Result<bool> {
        unsafe {
            let menu = CreatePopupMenu()?;
            let build_menu = || -> Result<()> {
                add_item(menu, 1, "自由活动", self.motion.mode() == Mode::Auto)?;
                add_item(menu, 2, "原地休整", self.motion.mode() == Mode::Rest)?;
                AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                add_item(
                    menu,
                    34,
                    &format!("大小 {}%…", self.settings.size_percent),
                    false,
                )?;
                for (id, speed, label) in [
                    (11, 1.0, "速度 1×"),
                    (12, 1.5, "速度 1.5×"),
                    (13, 2.0, "速度 2×"),
                ] {
                    add_item(menu, id, label, (self.motion.speed() - speed).abs() < 0.01)?;
                }
                AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                let (limit, effective) = frame_rates(self.settings.fps, self.monitor_hz);
                for (id, fps) in [(20, 60), (21, 120), (22, 144)] {
                    let mut label = format!("帧率上限 {fps} FPS");
                    if fps == limit && effective < limit {
                        label.push_str(&format!("（当前屏幕 {effective} Hz）"));
                    }
                    add_item(menu, id, &label, fps == limit)?;
                }
                AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                add_item(menu, 30, "暂停", self.settings.paused)?;
                add_item(
                    menu,
                    31,
                    "鼠标穿透（从托盘恢复）",
                    self.settings.click_through,
                )?;
                add_item(menu, 32, "全屏应用时隐藏", self.settings.fullscreen)?;
                add_item(menu, 33, "移回屏幕中央", false)?;
                AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                #[cfg(all(feature = "standalone", target_arch = "x86_64"))]
                add_item(
                    menu,
                    35,
                    concat!("检查更新（v", env!("CARGO_PKG_VERSION"), "）"),
                    false,
                )?;
                add_item(menu, 99, "退出鸽子", false)
            };
            if let Err(error) = build_menu() {
                let _ = DestroyMenu(menu);
                return Err(error);
            }
            let mut pt = POINT::default();
            GetCursorPos(&mut pt)?;
            let _ = SetForegroundWindow(self.hwnd);
            let command = TrackPopupMenuEx(
                menu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
                pt.x,
                pt.y,
                self.hwnd,
                None,
            )
            .0 as u32;
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            match command {
                1 | 2 => {
                    self.settings.mode = if command == 1 { Mode::Auto } else { Mode::Rest };
                    self.motion.set_mode(self.settings.mode);
                    self.settings.paused = false;
                }
                11..=13 => {
                    self.settings.speed = [1.0, 1.5, 2.0][(command - 11) as usize];
                    self.motion.set_speed(self.settings.speed);
                }
                20..=22 => self.settings.fps = Some([60, 120, 144][(command - 20) as usize]),
                30 => self.settings.paused = !self.settings.paused,
                31 => {
                    self.settings.click_through = !self.settings.click_through;
                    self.show_body();
                }
                32 => self.settings.fullscreen = !self.settings.fullscreen,
                33 => {
                    self.center();
                    self.position()?;
                }
                34 => {
                    GetCursorPos(&mut pt)?;
                    size_slider::show(self.hwnd, pt, self.settings.size_percent)?;
                }
                #[cfg(all(feature = "standalone", target_arch = "x86_64"))]
                35 => crate::updater::check_now(),
                99 => return Ok(false),
                _ => {}
            }
            Ok(true)
        }
    }
}

unsafe fn add_item(menu: HMENU, id: usize, label: &str, checked: bool) -> Result<()> {
    let text: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
    unsafe {
        AppendMenuW(
            menu,
            MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED },
            id,
            PCWSTR(text.as_ptr()),
        )
    }
}

unsafe fn monitor_settings(hwnd: HWND) -> Result<(RECT, Option<u32>)> {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        GetMonitorInfoW(monitor, &mut info.monitorInfo).ok()?;
        let mut mode = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let valid = EnumDisplaySettingsW(
            PCWSTR(info.szDevice.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut mode,
        )
        .as_bool();
        let hz = (valid && mode.dmDisplayFrequency > 1).then_some(mode.dmDisplayFrequency);
        Ok((info.monitorInfo.rcWork, hz))
    }
}

// Windows positions popups on whole pixels. Draw the remaining fraction inside both windows.
fn split_pixel(value: f32) -> (i32, f32) {
    let pixel = value.round();
    (pixel as i32, value - pixel)
}

fn sprite_origin(base_scale: f32, scale: f32, offset: (f32, f32)) -> (f32, f32) {
    // The ground beneath the feet stays put as the bird gets taller or shorter.
    (
        WIDTH * 0.5 * base_scale + offset.0,
        145.0 * base_scale + GROUND_Y * (base_scale - scale) + offset.1,
    )
}

fn frame_rates(preference: Option<u32>, monitor_hz: Option<u32>) -> (u32, u32) {
    let limit = preference.unwrap_or(match monitor_hz {
        Some(144..) => 144,
        Some(120..) => 120,
        _ => 60,
    });
    (limit, monitor_hz.unwrap_or(limit).min(limit))
}

unsafe fn fullscreen_on_monitor(hwnd: HWND) -> bool {
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground == hwnd || foreground.0.is_null() {
            return false;
        }
        let mut class = [0u16; 128];
        let n = GetClassNameW(foreground, &mut class);
        let class = String::from_utf16_lossy(&class[..n as usize]);
        if matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return false;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST) != monitor {
            return false;
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mut rect = RECT::default();
        if !GetMonitorInfoW(monitor, &mut info).as_bool()
            || GetWindowRect(foreground, &mut rect).is_err()
        {
            return false;
        }
        rect.left <= info.rcMonitor.left + 1
            && rect.top <= info.rcMonitor.top + 1
            && rect.right >= info.rcMonitor.right - 1
            && rect.bottom >= info.rcMonitor.bottom - 1
    }
}

pub fn run() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let result = run_inner();
        CoUninitialize();
        result
    }
}

unsafe fn run_inner() -> Result<()> {
    unsafe {
        let arguments: Vec<String> = std::env::args().collect();
        let inspect = arguments.iter().any(|s| s == "--inspect");
        let smoke = arguments
            .iter()
            .position(|s| s == "--smoke-test")
            .and_then(|i| arguments.get(i + 1))
            .cloned();
        let _instance = OwnedHandle(CreateMutexW(
            None,
            false,
            if smoke.is_some() {
                w!("Local\\Pigeon.Native.SmokeTest")
            } else {
                w!("Local\\Pigeon.Native.Prototype")
            },
        )?);
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(());
        }
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let body_class = WNDCLASSW {
            lpfnWndProc: Some(body_proc),
            lpszClassName: BODY_CLASS,
            ..class
        };
        if RegisterClassW(&body_class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let scale = GetDpiForSystem() as f32 / 96.0 * 1.15;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_TOPMOST
                | if inspect {
                    WS_EX_APPWINDOW
                } else {
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
                },
            CLASS,
            TITLE,
            WS_POPUP,
            0,
            0,
            (WIDTH * scale) as i32,
            (HEIGHT * scale) as i32,
            None,
            None,
            Some(module.into()),
            None,
        )?;
        let graphics = match Graphics::new(hwnd, (WIDTH * scale) as u32, (HEIGHT * scale) as u32) {
            Ok(graphics) => graphics,
            Err(error) => {
                let _ = DestroyWindow(hwnd);
                return Err(error);
            }
        };
        let tray = match Tray::new(hwnd) {
            Ok(tray) => tray,
            Err(error) => {
                let _ = DestroyWindow(hwnd);
                return Err(error);
            }
        };
        let (work, monitor_hz) = monitor_settings(hwnd)?;
        // A surface-less owned window receives input without clipping the drawn feathers.
        let body = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            BODY_CLASS,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            (WIDTH * scale) as i32,
            (HEIGHT * scale) as i32,
            Some(hwnd),
            None,
            Some(module.into()),
            None,
        )?;
        // Diagnostic runs use their own defaults and never edit desktop preferences.
        let settings_path = if inspect || smoke.is_some() {
            None
        } else {
            Settings::path()
        };
        let mut settings = settings_path
            .as_deref()
            .map(Settings::load)
            .unwrap_or_default();
        if smoke.is_some() {
            settings.fps = Some(144);
        }
        let mut motion = Motion::new();
        motion.set_mode(if inspect { Mode::Rest } else { settings.mode });
        motion.set_speed(settings.speed);
        let mut app = App {
            hwnd,
            body,
            graphics,
            _tray: tray,
            motion,
            base_scale: scale,
            settings,
            work,
            monitor_hz,
            hidden: settings.fullscreen && fullscreen_on_monitor(hwnd),
            drag: None,
            last_position: None,
            offset: (0.0, 0.0),
        };
        app.center();
        if smoke.is_some() && arguments.iter().any(|s| s == "--smoke-half-speed") {
            app.motion.set_speed(0.5);
        }
        app.motion.update(0.0, app.bound_width());
        app.position()?;
        if !app.hidden {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            app.render()?;
        }
        #[cfg(all(feature = "standalone", target_arch = "x86_64"))]
        if !inspect && smoke.is_none() {
            crate::updater::start();
        }
        let timer = OwnedHandle(
            CreateWaitableTimerExW(
                None,
                None,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
            .or_else(|_| CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0))?,
        );
        let started = Instant::now();
        let mut saved_settings = settings;
        let mut save_after = started;
        let mut save_warning_shown = false;
        let mut last = started;
        let mut deadline = started;
        let mut environment = started - Duration::from_secs(1);
        let mut frames = 0u64;
        let mut ticks = 0u64;
        let mut skipped_duplicates = 0u64;
        let mut walk_frames = 0u64;
        let mut pose_frames = [0u32; 2];
        let mut pose_seconds = [0.0f32; 2];
        let mut previous_pixels = Vec::new();
        let mut previous_facing = 0.0;
        let mut last_present = started;
        let mut max_tick_gap = 0.0f32;
        let mut max_gap = 0.0f32;
        let mut running = true;
        while running {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    running = false;
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let events = EVENTS.with(|q| std::mem::take(&mut *q.borrow_mut()));
            for event in events {
                match event {
                    Event::Menu => {
                        running = app.popup()?;
                        last = Instant::now();
                        deadline = last;
                        save_after = last;
                    }
                    Event::Size(percent) => {
                        app.set_size(percent)?;
                        // Combine thumb movements into one write once the slider settles.
                        save_after = Instant::now() + Duration::from_millis(250);
                    }
                    Event::Close => running = false,
                    Event::Down(pt) => app.drag = Some((pt, app.motion.x(), app.motion.ground())),
                    Event::Move(pt) => {
                        if let Some((start, x, y)) = app.drag {
                            app.motion.set_x(
                                (x + (pt.x - start.x) as f32 / app.scale())
                                    .clamp(0.0, app.bound_width()),
                            );
                            app.motion.set_ground(
                                y + (pt.y - start.y) as f32 / app.scale(),
                                app.bound_height(),
                            );
                            app.position()?;
                        }
                    }
                    Event::Up => app.drag = None,
                    Event::Display => {
                        app.graphics.last_presented = None;
                        let old_top = app.work.top;
                        (app.work, app.monitor_hz) = monitor_settings(hwnd)?;
                        app.motion
                            .set_x(app.motion.x().clamp(0.0, app.bound_width()));
                        app.motion.set_ground(
                            app.motion.ground() + (old_top - app.work.top) as f32 / app.scale(),
                            app.bound_height(),
                        );
                        app.position()?;
                    }
                }
            }
            let now = Instant::now();
            if let Some(path) = settings_path.as_deref()
                && app.settings != saved_settings
                && (!running || now >= save_after)
            {
                match app.settings.save(path) {
                    Ok(()) => {
                        saved_settings = app.settings;
                        save_warning_shown = false;
                    }
                    Err(error) => {
                        if !save_warning_shown {
                            report_error(&format!(
                                "无法保存鸽子设置到 {}\n{error}",
                                path.display()
                            ));
                            save_warning_shown = true;
                        }
                        save_after = Instant::now() + Duration::from_secs(30);
                    }
                }
            }
            if !running {
                break;
            }
            if now.duration_since(environment) >= Duration::from_millis(500) {
                let hidden = app.settings.fullscreen && fullscreen_on_monitor(hwnd);
                if hidden != app.hidden {
                    app.hidden = hidden;
                    app.graphics.last_presented = None;
                    let _ = ShowWindow(hwnd, if hidden { SW_HIDE } else { SW_SHOWNOACTIVATE });
                    app.show_body();
                }
                environment = now;
            }
            if let Some(path) = &smoke {
                let elapsed = now.duration_since(started).as_secs_f32();
                let mode = if elapsed < 4.0 {
                    Mode::Walk
                } else if elapsed < 8.0 {
                    Mode::Look
                } else if elapsed < 12.0 {
                    Mode::Peck
                } else {
                    Mode::Auto
                };
                if app.motion.mode() != mode {
                    app.motion.set_mode(mode);
                }
                if elapsed >= 16.0 {
                    let report = format!(
                        "duration_seconds={elapsed:.3}\ntarget_fps={}\nrendered_frames={frames}\naverage_fps={:.2}\nmax_frame_gap_ms={:.2}\nanimation_ticks={ticks}\naverage_tick_hz={:.2}\nmax_tick_gap_ms={:.2}\nskipped_duplicates={skipped_duplicates}\nwalk_average_fps={:.2}\nlook_pose_fps={:.2}\npeck_pose_fps={:.2}\nspeed={:.1}\n",
                        frame_rates(app.settings.fps, app.monitor_hz).1,
                        frames as f32 / elapsed,
                        max_gap * 1000.0,
                        ticks as f32 / elapsed,
                        max_tick_gap * 1000.0,
                        walk_frames as f32 / 4.0,
                        pose_frames[0] as f32 / pose_seconds[0].max(f32::EPSILON),
                        pose_frames[1] as f32 / pose_seconds[1].max(f32::EPSILON),
                        app.motion.speed(),
                    );
                    std::fs::write(path, report).map_err(|e| {
                        windows::core::Error::new(E_FAIL, format!("Cannot write diagnostics: {e}"))
                    })?;
                    break;
                }
            }
            if now >= deadline {
                let dt = now.duration_since(last).as_secs_f32();
                last = now;
                if !app.hidden && (!app.settings.paused || app.graphics.last_presented.is_none()) {
                    if !app.settings.paused && app.drag.is_none() {
                        app.motion.update(dt.min(0.1), app.bound_width());
                        app.position()?;
                    }
                    ticks += 1;
                    max_tick_gap = max_tick_gap.max(dt);
                    let presentation = app.render()?;
                    if smoke.is_some() {
                        let index = match (app.motion.mode(), app.motion.sprite_clip().0) {
                            (Mode::Look, "look") => Some(0),
                            (Mode::Peck, "peck") => Some(1),
                            _ => None,
                        };
                        if let Some(index) = index {
                            pose_seconds[index] += dt;
                            if matches!(presentation, Presentation::Submitted) {
                                let pixels = app.graphics.pigeon.frame_pixels();
                                let facing = app.motion.facing();
                                if previous_pixels != pixels || previous_facing != facing {
                                    pose_frames[index] += 1;
                                    previous_pixels.clear();
                                    previous_pixels.extend_from_slice(pixels);
                                    previous_facing = facing;
                                }
                            }
                        }
                    }
                    match presentation {
                        Presentation::Unchanged => skipped_duplicates += 1,
                        Presentation::Submitted => {
                            frames += 1;
                            if smoke.is_some()
                                && now.duration_since(started) < Duration::from_secs(4)
                            {
                                walk_frames += 1;
                            }
                            let submitted = Instant::now();
                            max_gap =
                                max_gap.max(submitted.duration_since(last_present).as_secs_f32());
                            last_present = submitted;
                        }
                        Presentation::Unavailable => {}
                    }
                }
                let period = if app.hidden || app.settings.paused {
                    Duration::from_millis(200)
                } else {
                    let (_, effective) = frame_rates(app.settings.fps, app.monitor_hz);
                    Duration::from_secs_f64(1.0 / effective as f64)
                };
                deadline += period;
                // Recover from a late start without charging the next tick for time spent rendering.
                if deadline <= now {
                    deadline = now + period;
                }
            }
            let wait = deadline.saturating_duration_since(Instant::now());
            if wait.is_zero() {
                continue;
            }
            let due = -((wait.as_nanos() / 100).max(1).min(i64::MAX as u128) as i64);
            SetWaitableTimerEx(timer.0, &due, 0, None, None, None, 0)?;
            let result = MsgWaitForMultipleObjectsEx(
                Some(&[timer.0]),
                INFINITE,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
            if result == WAIT_FAILED {
                return Err(windows::core::Error::from_thread());
            }
        }
        Ok(())
    }
}

pub fn report_error(message: &str) {
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(None, PCWSTR(text.as_ptr()), TITLE, MB_OK | MB_ICONERROR);
    }
}

#[cfg(test)]
mod tests {
    use super::{GROUND_Y, frame_rates, split_pixel, sprite_origin};

    #[test]
    fn frame_limit_survives_changes_to_the_display_refresh_rate() {
        assert_eq!(frame_rates(Some(144), Some(120)), (144, 120));
        assert_eq!(frame_rates(Some(144), Some(165)), (144, 144));
        assert_eq!(frame_rates(Some(60), Some(120)), (60, 60));
        assert_eq!(frame_rates(Some(120), None), (120, 120));
        assert_eq!(frame_rates(None, Some(120)), (120, 120));
    }

    #[test]
    fn changing_size_preserves_the_ground_anchor_and_scales_the_sprite() {
        let base = 1.725;
        let offset = (0.25, -0.375);
        let full = sprite_origin(base, base, offset);
        for percent in [50, 51, 73, 99, 100] {
            let scale = base * (percent as f32 / 100.0);
            let (x, y) = sprite_origin(base, scale, offset);
            assert_eq!(x, full.0);
            assert!((y + GROUND_Y * scale - (full.1 + GROUND_Y * base)).abs() < 0.001);
        }
    }

    #[test]
    fn slow_motion_crosses_native_window_pixels_continuously() {
        let positions = [2.0, 2.25, 2.5, 2.75, 3.0];
        for p in positions.into_iter().chain(positions.map(|p| -p)) {
            let (window, offset) = split_pixel(p);
            assert!(offset.abs() <= 0.5);
            assert_eq!(window as f32 + offset, p);
        }
    }
}
