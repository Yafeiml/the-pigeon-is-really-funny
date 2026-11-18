use std::{os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_FILENAME_EXCED_RANGE, ERROR_MORE_DATA,
            ERROR_PATH_NOT_FOUND, ERROR_UNSUPPORTED_TYPE,
        },
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
        },
    },
    core::{PCWSTR, Result, w},
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("ThePigeonIsReallyFunny");
// Windows documents a maximum of 260 characters for a Run command line.
const MAX_COMMAND: usize = 260;

fn command(exe: &Path) -> Vec<u16> {
    std::iter::once(b'"' as u16)
        .chain(exe.as_os_str().encode_wide())
        .chain([b'"' as u16, 0])
        .collect()
}

fn registered(key: PCWSTR, expected: &[u16]) -> Result<bool> {
    let mut stored = [0u16; MAX_COMMAND + 1];
    let mut bytes = size_of_val(&stored) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(stored.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    match status {
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_MORE_DATA | ERROR_UNSUPPORTED_TYPE => {
            Ok(false)
        }
        _ => {
            status.ok()?;
            Ok(stored.get(..bytes as usize / 2) == Some(expected))
        }
    }
}

fn write_registration(key: PCWSTR, enabled: bool, command: &[u16]) -> Result<()> {
    if enabled {
        if command.len() > MAX_COMMAND + 1 {
            return ERROR_FILENAME_EXCED_RANGE.ok();
        }
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key,
                VALUE,
                REG_SZ.0,
                Some(command.as_ptr().cast()),
                size_of_val(command) as u32,
            )
            .ok()
        }
    } else {
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key, VALUE) } {
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(()),
            status => status.ok(),
        }
    }
}

pub fn enabled() -> Result<bool> {
    registered(RUN_KEY, &command(&std::env::current_exe()?))
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    write_registration(RUN_KEY, enabled, &command(&std::env::current_exe()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        Win32::System::{Com::CoCreateGuid, Registry::RegDeleteKeyW},
        core::HSTRING,
    };

    #[test]
    fn login_registration_quotes_the_current_exe_and_can_be_relocated_and_removed() {
        let key_name = HSTRING::from(format!(
            "Software\\DesktopPigeonAutostartTest-{:?}",
            unsafe { CoCreateGuid() }.unwrap()
        ));
        let key = PCWSTR(key_name.as_ptr());
        let first = command(Path::new(r"C:\鸽子 小屋\pigeon.exe"));
        let moved = command(Path::new(r"C:\桌面\The pigeon.exe"));
        assert_eq!(
            first,
            "\"C:\\鸽子 小屋\\pigeon.exe\"\0"
                .encode_utf16()
                .collect::<Vec<_>>()
        );
        assert!(!registered(key, &first).unwrap());
        write_registration(key, true, &first).unwrap();
        assert!(registered(key, &first).unwrap());
        assert!(!registered(key, &moved).unwrap());
        write_registration(key, true, &moved).unwrap();
        assert!(registered(key, &moved).unwrap());
        assert!(write_registration(key, true, &[65; MAX_COMMAND + 2]).is_err());
        assert!(registered(key, &moved).unwrap());
        write_registration(key, false, &moved).unwrap();
        write_registration(key, false, &moved).unwrap();
        assert!(!registered(key, &moved).unwrap());
        unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, key) }
            .ok()
            .unwrap();
    }
}
