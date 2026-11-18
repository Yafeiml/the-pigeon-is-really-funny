use std::os::windows::io::{FromRawHandle, OwnedHandle};
use windows::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, SetLastError},
        System::Threading::{CreateMutexExW, SYNCHRONIZATION_SYNCHRONIZE},
    },
    core::{PCWSTR, Result, w},
};

pub(crate) fn acquire() -> Result<Option<OwnedHandle>> {
    // Keep the original name so upgrades and renamed copies share one desktop pet.
    acquire_named(w!("Local\\Pigeon.Native.Prototype"))
}

fn acquire_named(name: PCWSTR) -> Result<Option<OwnedHandle>> {
    unsafe {
        SetLastError(ERROR_SUCCESS);
        let raw = CreateMutexExW(None, name, 0, SYNCHRONIZATION_SYNCHRONIZE.0)?;
        let exists = GetLastError() == ERROR_ALREADY_EXISTS;
        let handle = OwnedHandle::from_raw_handle(raw.0);
        Ok((!exists).then_some(handle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::HSTRING;

    #[test]
    fn duplicate_launches_exit_until_the_original_handle_closes() {
        let name = HSTRING::from(format!(
            "Local\\Pigeon.Test.Instance.{}",
            std::process::id()
        ));
        let first = acquire_named(PCWSTR(name.as_ptr())).unwrap().unwrap();
        std::thread::scope(|threads| {
            for _ in 0..12 {
                threads.spawn(|| assert!(acquire_named(PCWSTR(name.as_ptr())).unwrap().is_none()));
            }
        });
        drop(first);
        assert!(acquire_named(PCWSTR(name.as_ptr())).unwrap().is_some());
    }
}
