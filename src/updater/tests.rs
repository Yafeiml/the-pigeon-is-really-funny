use super::*;
use serde_json::json;
use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
};
use windows::Win32::System::Com::CoCreateGuid;

#[test]
fn only_an_exact_new_stable_release_is_eligible() {
    let asset = json!({
        "name": ASSET, "browser_download_url": format!("https://github.com/{REPO}/releases/download/v0.10.0/{ASSET}"),
        "size": 512, "digest": format!("sha256:{}", "f".repeat(64)), "state": "uploaded"
    });
    let good = json!({"tag_name":"v0.10.0","draft":false,"prerelease":false,"assets":[asset]});
    let current = Version::parse("0.2.0").unwrap();
    let parse = |doc: &Value| release(&serde_json::to_vec(doc).unwrap(), &current);
    assert_eq!(parse(&good).unwrap().unwrap().version.to_string(), "0.10.0");
    for (pointer, replacement) in [
        ("/tag_name", json!("v0.10.1-rc.1")),
        ("/tag_name", json!("v0.10.1+ci")),
        ("/prerelease", json!(true)),
        ("/draft", json!(true)),
        ("/assets/0/state", json!("open")),
        ("/assets/0/name", json!("another.exe")),
        (
            "/assets/0/browser_download_url",
            json!("https://github.com/elsewhere/repo/releases/download/v0.10.0/pigeon.exe"),
        ),
        ("/assets/0/digest", Value::Null),
        ("/assets/0/size", json!(1_073_741_824)),
    ] {
        let mut doc = good.clone();
        *doc.pointer_mut(pointer).unwrap() = replacement;
        assert!(parse(&doc).is_err(), "{pointer}");
    }
    let mut duplicated = good.clone();
    duplicated["assets"].as_array_mut().unwrap().push(asset);
    assert!(parse(&duplicated).is_err());
    for tag in ["v0.1.0", "v0.2.0"] {
        let mut old = good.clone();
        old["tag_name"] = json!(tag);
        assert!(parse(&old).unwrap().is_none());
    }
}

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("pigeon-updater-test-{:?}", unsafe {
            CoCreateGuid().unwrap()
        }));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        // Remove only the individual files created by these tests, even on panic.
        let exe = self.0.join("我的鸽子 有空格.exe");
        let work = work_dir(&exe).unwrap();
        for file in ["incoming.exe", "previous.exe", "version", "owner.lock"] {
            let _ = fs::remove_file(work.join(file));
        }
        let _ = fs::remove_dir(work);
        let _ = fs::remove_file(exe);
        let _ = fs::remove_dir(&self.0);
    }
}

#[test]
fn failed_download_or_probe_preserves_the_installed_file() {
    let test = Sandbox::new();
    let exe = test.0.join("我的鸽子 有空格.exe");
    fs::write(&exe, b"previous binary").unwrap();
    let bytes = b"invalid executable";
    let mut asset = Asset {
        version: Version::parse("0.2.0").unwrap(),
        url: String::new(),
        size: bytes.len() as u64,
        hash: format!("{:x}", Sha256::digest(bytes)),
    };
    for data in [&bytes[..bytes.len() - 1], b"invalid executablf", &bytes[..]] {
        assert!(install(&exe, data, &asset).is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"previous binary");
        let work = work_dir(&exe).unwrap();
        assert!(!work.join("incoming.exe").exists());
        assert!(!work.join("previous.exe").exists());
    }
    asset.size -= 1;
    assert!(install(&exe, bytes.as_slice(), &asset).is_err());
    assert_eq!(fs::read(exe).unwrap(), b"previous binary");
}

#[test]
fn rollback_preserves_last_working_copy_on_replace_failure() {
    let test = Sandbox::new();
    let exe = test.0.join("我的鸽子 有空格.exe");
    let work = Workspace::open(work_dir(&exe).unwrap()).unwrap();
    let incoming = work.dir.join("incoming.exe");
    let backup = work.dir.join("previous.exe");
    fs::write(&exe, b"old").unwrap();
    fs::write(&incoming, b"new").unwrap();
    let locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&exe)
        .unwrap();
    assert!(replace(&exe, &incoming, &backup).is_err());
    drop(locked);
    assert_eq!(fs::read(&exe).unwrap(), b"old");
    // Reproduce the documented intermediate state in which the old image has
    // already acquired its backup name and the new file is unavailable.
    fs::rename(&exe, &backup).unwrap();
    fs::remove_file(&incoming).unwrap();
    assert!(replace(&exe, &incoming, &backup).is_err());
    assert_eq!(fs::read(&exe).unwrap(), b"old");
}

#[test]
#[ignore = "requires a packaged Windows executable at PIGEON_TEST_STANDALONE_EXE"]
fn live_image_replacement_and_restart() {
    let candidate = PathBuf::from(std::env::var_os("PIGEON_TEST_STANDALONE_EXE").unwrap());
    let test = Sandbox::new();
    let exe = test.0.join("我的鸽子 有空格.exe");
    let ping = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/ping.exe");
    fs::copy(&ping, &exe).unwrap();
    let mut old = Command::new(&exe)
        .args(["-n", "60", "127.0.0.1"])
        .creation_flags(0x08000000)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let result = (|| -> Result<()> {
        let mut hasher = Sha256::new();
        io::copy(&mut File::open(&candidate)?, &mut hasher)?;
        let asset = Asset {
            version: Version::parse(BUILD)?,
            url: String::new(),
            size: fs::metadata(&candidate)?.len(),
            hash: format!("{:x}", hasher.finalize()),
        };
        install(&exe, File::open(&candidate)?, &asset)?;
        assert!(old.try_wait()?.is_none());
        let work = work_dir(&exe)?;
        assert_eq!(fs::read(work.join("previous.exe"))?, fs::read(&ping)?);
        crate::pigeon::verify_update(&exe, &asset.version.to_string())?;
        Ok(())
    })();
    let _ = old.kill();
    let _ = old.wait();
    result.unwrap();
    let work = Workspace::open(work_dir(&exe).unwrap()).unwrap();
    work.cleanup(BUILD).unwrap();
    assert!(!work.dir.join("previous.exe").exists());
}

#[test]
#[ignore = "live verification after publishing the first release"]
fn public_release_can_be_read_and_downloaded() {
    let mut doc = vec![];
    http::get(
        &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        false,
    )
    .unwrap()
    .take(1 << 20)
    .read_to_end(&mut doc)
    .unwrap();
    let asset = release(&doc, &Version::parse("0.0.0").unwrap())
        .unwrap()
        .unwrap();
    let test = Sandbox::new();
    let exe = test.0.join("我的鸽子 有空格.exe");
    let mut file = File::create(&exe).unwrap();
    copy_verified(http::get(&asset.url, true).unwrap(), &mut file, &asset).unwrap();
    drop(file);
    crate::pigeon::verify_update(&exe, &asset.version.to_string()).unwrap();
}
