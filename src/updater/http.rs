use super::{Result, invalid};
use std::{ffi::c_void, io, ptr};
use windows::{
    Win32::Networking::WinHttp::*,
    core::{HSTRING, PCWSTR, w},
};

struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void) -> Result<Self> {
        if raw.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self(raw))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

pub(super) struct Response {
    // Child handles close before their parents.
    request: Handle,
    _connection: Handle,
    _session: Handle,
}

fn secure_endpoint(url: &str) -> Result<(&str, &str)> {
    let authority = url
        .strip_prefix("https://")
        .ok_or_else(|| invalid("更新地址需要 HTTPS"))?;
    let (host, _) = authority
        .split_once('/')
        .ok_or_else(|| invalid("更新地址缺少路径"))?;
    if !matches!(
        host,
        "api.github.com"
            | "github.com"
            | "release-assets.githubusercontent.com"
            | "objects.githubusercontent.com"
    ) || url
        .bytes()
        .any(|byte| byte.is_ascii_control() || matches!(byte, b'\\' | b'#' | b' '))
    {
        return Err(invalid("更新地址不属于 GitHub 发布服务").into());
    }
    Ok((host, &authority[host.len()..]))
}

pub(super) fn get(url: &str, asset: bool) -> Result<Response> {
    let mut url = url.to_owned();
    let session = unsafe {
        let session = Handle::new(WinHttpOpen(
            w!("DesktopPigeon"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 10_000, 15_000, 15_000, 35_000)?;
        session
    };
    for hop in 0..=4 {
        let (host, path) = secure_endpoint(&url)?;
        unsafe {
            let connection = Handle::new(WinHttpConnect(session.0, &HSTRING::from(host), 443, 0))?;
            let request = Handle::new(WinHttpOpenRequest(
                connection.0,
                w!("GET"),
                &HSTRING::from(path),
                PCWSTR::null(),
                PCWSTR::null(),
                ptr::null(),
                WINHTTP_FLAG_SECURE,
            ))?;
            WinHttpSetOption(
                Some(request.0),
                WINHTTP_OPTION_DISABLE_FEATURE,
                Some(&WINHTTP_DISABLE_REDIRECTS.to_ne_bytes()),
            )?;
            WinHttpSendRequest(request.0, None, None, 0, 0, 0)?;
            WinHttpReceiveResponse(request.0, ptr::null_mut())?;
            let mut status = 0u32;
            let mut len = size_of_val(&status) as u32;
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some((&raw mut status).cast()),
                &mut len,
                ptr::null_mut(),
            )?;
            if status == 200 {
                return Ok(Response {
                    request,
                    _connection: connection,
                    _session: session,
                });
            }
            if !asset || !matches!(status, 301 | 302 | 303 | 307 | 308) || hop == 4 {
                return Err(invalid(format!("GitHub 返回 HTTP {status}")).into());
            }
            let mut location = [0u16; 8192];
            let mut bytes = size_of_val(&location) as u32;
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_LOCATION,
                PCWSTR::null(),
                Some(location.as_mut_ptr().cast()),
                &mut bytes,
                ptr::null_mut(),
            )?;
            let redirect = String::from_utf16(&location[..bytes as usize / 2])?;
            // A release asset may redirect to GitHub's signed object storage URL.
            let (host, _) = secure_endpoint(&redirect)?;
            if !matches!(
                host,
                "release-assets.githubusercontent.com" | "objects.githubusercontent.com"
            ) {
                return Err(invalid("发布文件的跳转地址异常").into());
            }
            url = redirect;
        }
    }
    unreachable!()
}

impl io::Read for Response {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut bytes = 0;
        unsafe {
            WinHttpReadData(
                self.request.0,
                buf.as_mut_ptr().cast(),
                buf.len().min(u32::MAX as usize) as u32,
                &mut bytes,
            )
        }
        .map_err(io::Error::other)?;
        Ok(bytes as usize)
    }
}
