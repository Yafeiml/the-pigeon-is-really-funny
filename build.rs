use std::{env, fs, io, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    for file in ["pigeon.rc", "pigeon.ico", "tray-light.ico", "tray-dark.ico"] {
        println!("cargo:rerun-if-changed=src/icons/{file}");
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let output = out.join(if msvc { "icons.res" } else { "icons.o" });
    let mut tools = Vec::new();
    if msvc {
        tools.push(PathBuf::from("rc.exe"));
        // Cargo can find Visual Studio from an ordinary terminal with no SDK tools on PATH.
        if let Some(programs) = env::var_os("ProgramFiles(x86)") {
            let bin = PathBuf::from(programs).join("Windows Kits/10/bin");
            if let Ok(entries) = fs::read_dir(bin) {
                let mut versions: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
                versions.sort_unstable();
                tools.extend(versions.into_iter().rev().map(|v| v.join("x86/rc.exe")));
            }
        }
    } else {
        let arch = env::var("TARGET").unwrap();
        let arch = arch.split('-').next().unwrap();
        tools.push(format!("{arch}-w64-mingw32-windres").into());
        tools.push("windres".into());
    }
    for tool in tools {
        let mut cmd = Command::new(&tool);
        cmd.current_dir("src/icons");
        if msvc {
            cmd.args(["/nologo", "/fo"]).arg(&output).arg("pigeon.rc");
        } else {
            cmd.args(["-O", "coff", "-i", "pigeon.rc", "-o"])
                .arg(&output);
        }
        match cmd.status() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Ok(status) if status.success() => {
                println!("cargo:rustc-link-arg={}", output.display());
                return;
            }
            result => panic!("Failed to compile the Windows icons with {tool:?}: {result:?}"),
        }
    }
    panic!("Install the Windows SDK resource compiler (MSVC) or MinGW windres (GNU).");
}
