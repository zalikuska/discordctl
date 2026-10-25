//! Минимальный HTTPS-клиент на системном WinHTTP: только GET, только `https://`.
//!
//! Нужен команде `update`. Своя реализация вместо `curl.exe` или PowerShell: дочерние процессы, которые что-то
//! скачивают, антивирусные эвристики считают признаком дроппера (см. `winsys.rs`), а WinHTTP берёт у системы и
//! TLS, и настройки прокси.

use std::{io, ptr};
use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
    WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_REDIRECTS, WINHTTP_FLAG_SECURE,
    WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_LOCATION, WINHTTP_QUERY_STATUS_CODE,
};

type Handle = *mut core::ffi::c_void;

/// Закрывает хэндл WinHTTP при выходе из области видимости.
struct Guard(Handle);

impl Drop for Guard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn last_error(what: &str) -> io::Error {
    let e = io::Error::last_os_error();
    io::Error::new(e.kind(), format!("{what}: {e}"))
}

pub struct Reply {
    pub status: u32,
    /// Заголовок `Location` (если был).
    pub location: Option<String>,
    pub body: Vec<u8>,
}

/// GET `https://host/path`. `follow` — идти ли по перенаправлениям; `max_body` — потолок размера ответа в байтах.
pub fn get(url: &str, follow: bool, max_body: usize) -> io::Result<Reply> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "only https:// addresses are allowed"))?;
    let (host, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_owned()),
    };
    unsafe {
        let session = Guard(WinHttpOpen(
            wide("discordctl-update").as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        ));
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        // Разрешение имени, соединение, отправка, чтение (мс): зависшая сеть не должна вешать команду.
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 30_000, 60_000);
        let conn = Guard(WinHttpConnect(session.0, wide(host).as_ptr(), 443, 0));
        if conn.0.is_null() {
            return Err(last_error("WinHttpConnect"));
        }
        let req = Guard(WinHttpOpenRequest(
            conn.0,
            wide("GET").as_ptr(),
            wide(&path).as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err(last_error("WinHttpOpenRequest"));
        }
        if !follow {
            let flag = WINHTTP_DISABLE_REDIRECTS;
            WinHttpSetOption(req.0, WINHTTP_OPTION_DISABLE_FEATURE, &flag as *const u32 as *const _, 4);
        }
        if WinHttpSendRequest(req.0, ptr::null(), 0, ptr::null(), 0, 0, 0) == 0 {
            return Err(last_error("WinHttpSendRequest"));
        }
        if WinHttpReceiveResponse(req.0, ptr::null_mut()) == 0 {
            return Err(last_error("WinHttpReceiveResponse"));
        }

        let mut status = 0u32;
        let mut len = 4u32;
        if WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            &mut status as *mut u32 as *mut _,
            &mut len,
            ptr::null_mut(),
        ) == 0
        {
            return Err(last_error("WinHttpQueryHeaders"));
        }

        let mut location = None;
        let mut buf = vec![0u16; 2048];
        let mut bytes = (buf.len() * 2) as u32;
        if WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_LOCATION,
            ptr::null(),
            buf.as_mut_ptr() as *mut _,
            &mut bytes,
            ptr::null_mut(),
        ) != 0
        {
            location = Some(String::from_utf16_lossy(&buf[..(bytes as usize / 2).min(buf.len())]));
        }

        let mut body = Vec::new();
        loop {
            let mut avail = 0u32;
            if WinHttpQueryDataAvailable(req.0, &mut avail) == 0 {
                return Err(last_error("WinHttpQueryDataAvailable"));
            }
            if avail == 0 {
                break;
            }
            if body.len() + avail as usize > max_body {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "the response is larger than allowed"));
            }
            let at = body.len();
            body.resize(at + avail as usize, 0);
            let mut read = 0u32;
            if WinHttpReadData(req.0, body[at..].as_mut_ptr() as *mut _, avail, &mut read) == 0 {
                return Err(last_error("WinHttpReadData"));
            }
            body.truncate(at + read as usize);
        }
        Ok(Reply { status, location, body })
    }
}
