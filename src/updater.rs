//! Update only from the project's stable GitHub Releases. The running image can finish
//! normally: Windows keeps its mapped pages alive after ReplaceFileW moves it to backup.
mod http;
#[cfg(test)]
mod tests;

use semver::Version;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::{
        Storage::FileSystem::{REPLACE_FILE_FLAGS, ReplaceFileW},
        UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW},
    },
    core::{HSTRING, w},
};

const REPO: &str = "Yafeiml/the-pigeon-is-really-funny";
const ASSET: &str = "Pigeon-Windows-x64.exe";
const BUILD: &str = env!("CARGO_PKG_VERSION");
const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const MAGIC: &str = "DesktopPigeon GitHub updater v1\n";
const PENDING: &str = "新版已安装，下次启动鸽子时生效。";
static CHECKING: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

struct Asset {
    version: Version,
    url: String,
    size: u64,
    hash: String,
}

fn release(metadata: &[u8], current: &Version) -> Result<Option<Asset>> {
    let doc: Value = serde_json::from_slice(metadata)?;
    if doc["draft"] != false || doc["prerelease"] != false {
        return Err(invalid("发布版本状态异常").into());
    }
    let tag = doc["tag_name"]
        .as_str()
        .ok_or_else(|| invalid("发布版本缺少标签"))?;
    let version = Version::parse(
        tag.strip_prefix('v')
            .ok_or_else(|| invalid("发布标签需要 v 前缀"))?,
    )?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(invalid("发布标签需要稳定版本号").into());
    }
    if version <= *current {
        return Ok(None);
    }
    let mut matches = doc["assets"]
        .as_array()
        .ok_or_else(|| invalid("缺少发布文件"))?
        .iter()
        .filter(|item| item["name"] == ASSET);
    let entry = matches
        .next()
        .ok_or_else(|| invalid("未找到 Windows x64 发布文件"))?;
    let url = format!("https://github.com/{REPO}/releases/download/{tag}/{ASSET}");
    let hash = entry["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("发布文件缺少有效的 SHA-256 校验值"))?;
    let size = entry["size"]
        .as_u64()
        .filter(|n| (1..=512 * 1024 * 1024).contains(n))
        .ok_or_else(|| invalid("发布文件大小异常"))?;
    if matches.next().is_some()
        || entry["state"] != "uploaded"
        || entry["browser_download_url"] != url
    {
        return Err(invalid("发布文件来源或状态异常").into());
    }
    Ok(Some(Asset {
        version,
        url,
        size,
        hash: hash.to_ascii_lowercase(),
    }))
}

fn copy_verified(mut from: impl Read, to: &mut File, asset: &Asset) -> Result<()> {
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buf = [0u8; 64 * 1024];
    let started = Instant::now();
    loop {
        let n = from.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > asset.size || started.elapsed() > Duration::from_secs(20 * 60) {
            return Err(invalid("更新下载超出大小或时间限制").into());
        }
        hash.update(&buf[..n]);
        to.write_all(&buf[..n])?;
    }
    if total != asset.size || format!("{:x}", hash.finalize()) != asset.hash {
        return Err(invalid("更新文件大小或 SHA-256 校验失败").into());
    }
    to.sync_all()?;
    Ok(())
}

fn probe(exe: &Path, version: &Version) -> Result<()> {
    let mut process = Command::new(exe)
        .arg("--verify-assets")
        .arg(version.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn()?;
    let start = Instant::now();
    loop {
        if let Some(status) = process.try_wait()? {
            if status.success() {
                return Ok(());
            }
            return Err(invalid("新版程序未通过启动及动画资源验证").into());
        }
        if start.elapsed() > Duration::from_secs(90) {
            let _ = process.kill();
            let _ = process.wait();
            return Err(invalid("新版程序启动验证超时").into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

// A per-installation work directory on the same volume, with a no-sharing lock
// for simultaneous Windows sessions. Only our own three filenames are removed.
struct Workspace {
    dir: PathBuf,
    _lock: File,
}

fn work_dir(exe: &Path) -> Result<PathBuf> {
    let mut name = OsString::from(".");
    name.push(exe.file_name().ok_or_else(|| invalid("程序路径无效"))?);
    name.push(".pigeon-update");
    Ok(exe.with_file_name(name))
}

impl Workspace {
    fn open(dir: PathBuf) -> Result<Self> {
        let fresh = match fs::create_dir(&dir) {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(e.into()),
        };
        if fs::symlink_metadata(&dir)?.file_attributes() & 0x400 != 0 {
            return Err(invalid("更新工作目录是重解析点").into());
        }
        let mut lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(fresh)
            .share_mode(0)
            .open(dir.join("owner.lock"))?;
        if fresh {
            lock.write_all(MAGIC.as_bytes())?;
            lock.sync_all()?;
        } else {
            let mut marker = String::new();
            (&mut lock).take(128).read_to_string(&mut marker)?;
            if marker != MAGIC {
                return Err(invalid("更新工作目录已被占用").into());
            }
        }
        Ok(Self { dir, _lock: lock })
    }

    fn remove_partial(&self) -> io::Result<()> {
        match fs::remove_file(self.dir.join("incoming.exe")) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    fn cleanup(&self, running: &str) -> Result<()> {
        self.remove_partial()?;
        // Delete the old image only after the new version has reached its normal UI.
        let backup = self.dir.join("previous.exe");
        if backup.try_exists()? && fs::read_to_string(self.dir.join("version"))? == running {
            fs::remove_file(backup)?;
        }
        Ok(())
    }
}

fn replace(exe: &Path, candidate: &Path, backup: &Path) -> Result<()> {
    let result = unsafe {
        ReplaceFileW(
            &HSTRING::from(exe.as_os_str()),
            &HSTRING::from(candidate.as_os_str()),
            &HSTRING::from(backup.as_os_str()),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    };
    if let Err(e) = result {
        // ReplaceFileW documents a partial failure where the old image has already
        // moved to the backup but the replacement could not take its name.
        if !exe.try_exists()? && backup.try_exists()? {
            fs::rename(backup, exe)?;
        }
        return Err(e.into());
    }
    Ok(())
}

fn install(exe: &Path, source: impl Read, asset: &Asset) -> Result<()> {
    let work = Workspace::open(work_dir(exe)?)?;
    work.cleanup(BUILD)?;
    let previous = work.dir.join("previous.exe");
    if previous.try_exists()? {
        return Err(invalid("上一版备份仍在使用，请重新启动鸽子").into());
    }
    let candidate = work.dir.join("incoming.exe");
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&candidate)?;
        copy_verified(source, &mut file, asset)?;
        drop(file);
        probe(&candidate, &asset.version)?;
        let version_path = work.dir.join("version");
        if version_path.try_exists()? {
            fs::remove_file(&version_path)?;
        }
        let mut version_file = File::create_new(version_path)?;
        version_file.write_all(asset.version.to_string().as_bytes())?;
        version_file.sync_all()?;
        replace(exe, &candidate, &previous)
    })();
    let _ = work.remove_partial();
    result
}

fn check() -> Result<&'static str> {
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(PENDING);
    }
    let api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut metadata = vec![];
    const LIMIT: u64 = 1 << 20;
    http::get(&api, false)?
        .take(LIMIT + 1)
        .read_to_end(&mut metadata)?;
    if metadata.len() as u64 > LIMIT {
        return Err(invalid("发布信息过大").into());
    }
    let Some(asset) = release(&metadata, &Version::parse(BUILD)?)? else {
        return Ok("当前已经是最新稳定版。");
    };
    let response = http::get(&asset.url, true)?;
    install(&std::env::current_exe()?, response, &asset)?;
    INSTALLED.store(true, Ordering::Release);
    Ok(PENDING)
}

fn message(text: &str) {
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            w!("鸽子更新"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn check_and_notify(manual: bool) {
    if CHECKING.swap(true, Ordering::AcqRel) {
        if manual {
            message("正在检查更新。");
        }
        return;
    }
    if let Some(stamp) = stamp_path()
        && let Some(parent) = stamp.parent()
    {
        let _ = fs::create_dir_all(parent);
        let _ = fs::write(stamp, now().to_string());
    }
    let result = check();
    CHECKING.store(false, Ordering::Release);
    if manual {
        match result {
            Ok(text) => message(text),
            Err(e) => message(&format!("更新未完成。\n{e}")),
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn stamp_path() -> Option<PathBuf> {
    Some(
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
            .join("DesktopPigeon")
            .join("last-update-check"),
    )
}

pub fn start() {
    thread::spawn(|| {
        if let Ok(exe) = std::env::current_exe()
            && let Ok(dir) = work_dir(&exe)
            && dir.is_dir()
            && let Ok(work) = Workspace::open(dir)
        {
            let _ = work.cleanup(BUILD);
        }
        loop {
            let elapsed = stamp_path()
                .and_then(|stamp| fs::read_to_string(stamp).ok())
                .and_then(|date| now().checked_sub(date.parse().ok()?))
                .map(Duration::from_secs)
                .unwrap_or(DAY);
            if elapsed < DAY {
                thread::sleep(DAY - elapsed);
            }
            check_and_notify(false);
            if INSTALLED.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(DAY);
        }
    });
}

pub fn check_now() {
    thread::spawn(|| check_and_notify(true));
}
