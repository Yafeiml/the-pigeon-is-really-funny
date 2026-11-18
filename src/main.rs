#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod i18n;
mod motion;
#[cfg(windows)]
mod pigeon;
#[cfg(windows)]
mod single_instance;
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
            eprintln!(
                "{}",
                i18n::text(
                    "The executable version does not match the release tag.",
                    "程序版本与发布标签不一致。"
                )
            );
            std::process::exit(1);
        }
        if let Err(error) = pigeon::verify_embedded_assets() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let startup_error = |error| {
        windows_app::load_language();
        windows_app::report_error(&format!(
            "{}\n{error}",
            i18n::text("The pigeon could not start.", "鸽子启动失败。")
        ));
    };
    let _instance = match single_instance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => return startup_error(error),
    };
    if let Err(error) = windows_app::run() {
        startup_error(error);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The motion core is portable; this prototype currently needs Windows 10/11.");
}
