//! Named pipe: сервер (демон) и клиент (шим, CLI, MCP).
//!
//! Pipe доступен только текущему пользователю (DACL по SID) и не принимает удалённых клиентов.

use crate::protocol::{Envelope, Request, Response, TOKEN_ENV};
use std::{
    ffi::c_void,
    fmt,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    mem,
    os::windows::io::{FromRawHandle, RawHandle},
    ptr,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE},
    Security::{
        Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW},
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX},
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
            PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        },
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

/// Запрос короткий; ответ (история сообщений) может быть большим, но демону мы доверяем.
const MAX_LINE: u64 = 64 * 1024;
const MAX_REPLY: u64 = 64 * 1024 * 1024;
const ERROR_PIPE_BUSY: i32 = 231;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn user_sid() -> io::Result<String> {
    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut len);
        // u64 вместо u8: TOKEN_USER требует выравнивания по 8 байт.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as *mut c_void, len, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut ws: *mut u16 = ptr::null_mut();
        if ConvertSidToStringSidW(tu.User.Sid, &mut ws) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut n = 0;
        while *ws.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(ws, n));
        LocalFree(ws as _);
        Ok(s)
    }
}

pub struct Listener {
    name: Vec<u16>,
    sd: PSECURITY_DESCRIPTOR,
    next: HANDLE,
}

unsafe impl Send for Listener {}

impl Listener {
    /// Создаёт первый экземпляр pipe. Ошибка значит, что демон уже запущен.
    pub fn bind(name: &str) -> io::Result<Listener> {
        // Доступ: полный только текущему пользователю и SYSTEM; protected DACL, без наследования.
        let sddl = wide(&format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", user_sid()?));
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, ptr::null_mut()) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut l = Listener { name: wide(name), sd, next: ptr::null_mut() };
        l.next = l.create(true)?;
        Ok(l)
    }

    fn create(&self, first: bool) -> io::Result<HANDLE> {
        let sa = SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.sd,
            bInheritHandle: 0,
        };
        let mut open = PIPE_ACCESS_DUPLEX;
        if first {
            open |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        let h = unsafe {
            CreateNamedPipeW(self.name.as_ptr(), open, mode, PIPE_UNLIMITED_INSTANCES, 64 * 1024, 64 * 1024, 0, &sa)
        };
        if h == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(h)
        }
    }

    /// Ждёт клиента. Следующий экземпляр pipe создаётся сразу, чтобы клиенты не видели «pipe не найден».
    pub fn accept(&mut self) -> io::Result<File> {
        let h = self.next;
        let connected =
            unsafe { ConnectNamedPipe(h, ptr::null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !connected {
            let e = io::Error::last_os_error();
            unsafe { CloseHandle(h) };
            self.next = self.create(false)?;
            return Err(e);
        }
        self.next = self.create(false)?;
        Ok(unsafe { File::from_raw_handle(h as RawHandle) })
    }
}

pub fn read_request(conn: &File) -> io::Result<Envelope> {
    let mut line = String::new();
    BufReader::new(conn.take(MAX_LINE)).read_line(&mut line)?;
    serde_json::from_str(line.trim()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write_response(mut conn: &File, resp: &Response) -> io::Result<()> {
    let mut s = serde_json::to_string(resp).unwrap();
    s.push('\n');
    conn.write_all(s.as_bytes())?;
    conn.flush()?;
    // Ждём, пока клиент заберёт ответ, иначе закрытие хэндла может его обрезать.
    let _ = conn.sync_all();
    Ok(())
}

#[derive(Debug)]
pub enum ClientError {
    /// Pipe не существует: демон не запущен.
    NotRunning,
    Timeout,
    Io(io::Error),
    Bad(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::NotRunning => write!(f, "daemon is not running"),
            ClientError::Timeout => write!(f, "daemon did not answer in time"),
            ClientError::Io(e) => write!(f, "pipe error: {e}"),
            ClientError::Bad(e) => write!(f, "bad reply: {e}"),
        }
    }
}

pub fn request(pipe: &str, req: &Request, timeout: Duration) -> Result<Response, ClientError> {
    let (tx, rx) = mpsc::channel();
    let pipe = pipe.to_owned();
    // Токен приложения берётся из окружения: так им пользуются и CLI, и MCP-сервер без отдельных флагов.
    let token = std::env::var(TOKEN_ENV).ok().filter(|t| !t.is_empty());
    let line = serde_json::to_string(&Envelope { token, req: req.clone() }).unwrap();
    let deadline = Instant::now() + timeout;
    thread::spawn(move || {
        let _ = tx.send(do_request(&pipe, &line, deadline));
    });
    rx.recv_timeout(timeout).unwrap_or(Err(ClientError::Timeout))
}

fn do_request(pipe: &str, line: &str, deadline: Instant) -> Result<Response, ClientError> {
    let mut f = loop {
        match OpenOptions::new().read(true).write(true).open(pipe) {
            Ok(f) => break f,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(ClientError::NotRunning),
            Err(e) => return Err(ClientError::Io(e)),
        }
    };
    f.write_all(line.as_bytes()).and_then(|_| f.write_all(b"\n")).map_err(ClientError::Io)?;
    let mut reply = String::new();
    BufReader::new(f.take(MAX_REPLY)).read_line(&mut reply).map_err(ClientError::Io)?;
    serde_json::from_str(reply.trim()).map_err(|e| ClientError::Bad(e.to_string()))
}
