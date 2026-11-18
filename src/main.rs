#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod motion;
#[cfg(windows)]
mod pigeon;
#[cfg(all(windows, feature = "standalone", target_arch = "x86_64"))]
mod updater;
#[cfg(windows)]
mod windows_app;

#[cfg(windows)]
fn main() {
    #[cfg(feature = "standalone")]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--licenses")) {
        print!("{}", include_str!("../THIRD-PARTY-NOTICES.txt"));
        return;
    }
    #[cfg(feature = "standalone")]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--verify-assets")) {
        if let Some(expected) = std::env::args_os().nth(2)
            && expected != std::ffi::OsStr::new(env!("CARGO_PKG_VERSION"))
        {
            eprintln!("The executable version does not match the release tag.");
            std::process::exit(1);
        }
        if let Err(error) = pigeon::verify_embedded_assets() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = windows_app::run() {
        windows_app::report_error(&format!("鸽子启动失败：\n{error}"));
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The motion core is portable; this prototype currently needs Windows 10/11.");
}
