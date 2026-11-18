use std::mem::size_of;
use windows::{
    Win32::{
        Graphics::Gdi::{COLOR_BTNFACE, GetSysColor},
        System::{
            LibraryLoader::GetModuleHandleW,
            Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        },
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi},
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, Result, w},
};

#[derive(PartialEq, Eq, Clone, Copy)]
pub(super) struct TrayIcon {
    group: u16,
    edge: i32,
}

impl TrayIcon {
    pub(super) fn current() -> Self {
        unsafe {
            // The notification area and the pigeon may be on different monitors.
            let dpi = FindWindowW(w!("Shell_TrayWnd"), None)
                .map(|taskbar| GetDpiForWindow(taskbar))
                .unwrap_or(0);
            let dpi = if dpi == 0 { GetDpiForSystem() } else { dpi };
            Self {
                group: icon_group(),
                edge: GetSystemMetricsForDpi(SM_CXSMICON, dpi),
            }
        }
    }

    pub(super) fn load(self) -> Result<HICON> {
        unsafe {
            let image = LoadImageW(
                Some(GetModuleHandleW(None)?.into()),
                PCWSTR(self.group as usize as *const u16),
                IMAGE_ICON,
                self.edge,
                self.edge,
                LR_DEFAULTCOLOR,
            )?;
            Ok(HICON(image.0))
        }
    }
}

fn icon_group() -> u16 {
    unsafe {
        let mut contrast = HIGHCONTRASTW {
            cbSize: size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        if SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            Some((&raw mut contrast).cast()),
            Default::default(),
        )
        .is_ok()
            && contrast.dwFlags.contains(HCF_HIGHCONTRASTON)
        {
            let [r, g, b, _] = GetSysColor(COLOR_BTNFACE).to_le_bytes();
            let light = 299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) >= 128000;
            return if light { 2 } else { 3 };
        }
        let mut light = 0u32;
        let mut len = size_of::<u32>() as u32;
        let result = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut light).cast()),
            Some(&mut len),
        );
        // Group 1 is the neutral Explorer icon; groups 2 and 3 match the taskbar.
        match (result.is_ok(), light) {
            (true, 1) => 2,
            (true, 0) => 3,
            _ => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_loads_all_three_icon_groups_at_tray_sizes() {
        for group in 1..=3 {
            for edge in [16, 20, 24, 32, 48] {
                let icon = TrayIcon { group, edge }.load().unwrap();
                unsafe { DestroyIcon(icon) }.unwrap();
            }
        }
    }
}
