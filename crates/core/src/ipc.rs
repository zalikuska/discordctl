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
    os::windows::io::{AsRawHandle, FromRawHandle, RawHandle},
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_BROKEN_PIPE, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED,
        HANDLE, INVALID_HANDLE_VALUE,
    },
    Security::{
        Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW},
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX},
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PeekNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
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
    /// Запрос оборвали сами (`Abort`): агент отменил вызов.
    Aborted,
    Io(io::Error),
    Bad(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::NotRunning => write!(f, "daemon is not running"),
            ClientError::Timeout => write!(f, "daemon did not answer in time"),
            ClientError::Aborted => write!(f, "request was cancelled"),
            ClientError::Io(e) => write!(f, "pipe error: {e}"),
            ClientError::Bad(e) => write!(f, "bad reply: {e}"),
        }
    }
}

/// Флаг отмены для ожидающего запроса: после `abort()` клиент закрывает соединение, и демон видит, что отвечать некому
/// (`peer_gone`), а значит снимает вызов с ожидания подтверждения человека.
#[derive(Clone, Default)]
pub struct Abort(Arc<AtomicBool>);

impl Abort {
    pub fn abort(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Сколько байт ответа уже можно читать. `Err`, если другая сторона закрыла pipe.
fn available(f: &File) -> io::Result<u32> {
    let mut avail = 0u32;
    let ok = unsafe {
        PeekNamedPipe(f.as_raw_handle() as HANDLE, ptr::null_mut(), 0, ptr::null_mut(), &mut avail, ptr::null_mut())
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(avail)
    }
}

/// Закрыл ли клиент своё соединение. Демон спрашивает это, пока ждёт решения человека: запрос уже прочитан целиком,
/// поэтому ничего, кроме закрытия, с той стороны прийти не может. Неясная ошибка считается «клиент на месте».
pub fn peer_gone(conn: &File) -> bool {
    match available(conn) {
        Ok(_) => false,
        Err(e) => {
            matches!(e.raw_os_error(), Some(c) if c as u32 == ERROR_BROKEN_PIPE || c as u32 == ERROR_PIPE_NOT_CONNECTED)
        }
    }
}

pub fn request(pipe: &str, req: &Request, timeout: Duration) -> Result<Response, ClientError> {
    request_abortable(pipe, req, timeout, &Abort::default())
}

/// Как `request`, но вызов можно отменить из другого потока. Таймаут и отмена закрывают соединение, а не оставляют
/// фоновый поток висеть на чтении.
pub fn request_abortable(pipe: &str, req: &Request, timeout: Duration, abort: &Abort) -> Result<Response, ClientError> {
    // Токен приложения берётся из окружения: так им пользуются и CLI, и MCP-сервер без отдельных флагов.
    let token = std::env::var(TOKEN_ENV).ok().filter(|t| !t.is_empty());
    let line = serde_json::to_string(&Envelope { token, req: req.clone() }).unwrap();
    let deadline = Instant::now() + timeout;
    let mut f = loop {
        if abort.is_aborted() {
            return Err(ClientError::Aborted);
        }
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
    // Ждём ответ опросом, а не блокирующим чтением: так видны и таймаут, и отмена, причём до конца строки,
    // а не только до первого байта. Первые проверки без сна: быстрый ответ не должен ждать лишних миллисекунд.
    let mut reply: Vec<u8> = Vec::new();
    let mut idle = 0u32;
    let mut pause = Duration::from_millis(1);
    while !reply.ends_with(b"\n") && (reply.len() as u64) < MAX_REPLY {
        if abort.is_aborted() {
            return Err(ClientError::Aborted);
        }
        match available(&f) {
            Ok(n) if n > 0 => {
                let mut chunk = vec![0u8; (n as usize).min((MAX_REPLY as usize) - reply.len())];
                let got = f.read(&mut chunk).map_err(ClientError::Io)?;
                // Читаем до первого перевода строки: остальное в этом соединении не нужно.
                match chunk[..got].iter().position(|b| *b == b'\n') {
                    Some(i) => reply.extend_from_slice(&chunk[..=i]),
                    None => reply.extend_from_slice(&chunk[..got]),
                }
                idle = 0;
                pause = Duration::from_millis(1);
                continue;
            }
            Ok(_) => {}
            // Демон закрыл pipe, не договорив: разберём, что пришло.
            Err(_) if !reply.is_empty() => break,
            Err(e) => return Err(ClientError::Io(e)),
        }
        if Instant::now() >= deadline {
            return Err(ClientError::Timeout);
        }
        idle += 1;
        if idle < 100 {
            thread::yield_now();
        } else {
            thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_millis(10));
        }
    }
    serde_json::from_slice(&reply).map_err(|e| ClientError::Bad(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn pipe_name(tag: &str) -> String {
        format!(r"\\.\pipe\discordctl-test-{tag}-{}", std::process::id())
    }

    /// Клиент отменил ожидающий запрос: он получает `Aborted`, а сервер видит, что клиента больше нет.
    #[test]
    fn aborting_a_waiting_request_tells_the_server_the_client_is_gone() {
        let name = pipe_name("abort");
        let mut listener = Listener::bind(&name).unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let conn = listener.accept().unwrap();
            read_request(&conn).unwrap();
            assert!(!peer_gone(&conn), "the client is still waiting for the answer");
            tx.send(()).unwrap(); // запрос получен; дальше «ждём человека»
            let end = Instant::now() + Duration::from_secs(5);
            while !peer_gone(&conn) && Instant::now() < end {
                thread::sleep(Duration::from_millis(20));
            }
            tx.send(()).unwrap();
            assert!(peer_gone(&conn), "the server never noticed that the client hung up");
        });
        let abort = Abort::default();
        let a2 = abort.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            a2.abort();
        });
        let started = Instant::now();
        let r = request_abortable(&name, &Request::Ping, Duration::from_secs(10), &abort);
        assert!(matches!(r, Err(ClientError::Aborted)), "got {r:?}");
        assert!(started.elapsed() < Duration::from_secs(3), "must not wait for the timeout");
        canceller.join().unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap(); // сервер получил запрос
        rx.recv_timeout(Duration::from_secs(6)).expect("server did not notice the disconnect in time");
    }

    /// Таймаут тоже закрывает соединение, а не оставляет поток ждать вечно.
    #[test]
    fn a_timed_out_request_hangs_up() {
        let name = pipe_name("timeout");
        let mut listener = Listener::bind(&name).unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let conn = listener.accept().unwrap();
            read_request(&conn).unwrap();
            let end = Instant::now() + Duration::from_secs(5);
            while !peer_gone(&conn) && Instant::now() < end {
                thread::sleep(Duration::from_millis(20));
            }
            tx.send(peer_gone(&conn)).unwrap();
        });
        let r = request_abortable(&name, &Request::Ping, Duration::from_millis(300), &Abort::default());
        assert!(matches!(r, Err(ClientError::Timeout)), "got {r:?}");
        assert!(rx.recv_timeout(Duration::from_secs(6)).unwrap(), "server should see the client gone");
    }

    #[test]
    fn a_normal_request_still_gets_its_answer() {
        let name = pipe_name("ok");
        let mut listener = Listener::bind(&name).unwrap();
        thread::spawn(move || {
            let conn = listener.accept().unwrap();
            read_request(&conn).unwrap();
            write_response(&conn, &Response::ok("pong")).unwrap();
        });
        let r = request(&name, &Request::Ping, Duration::from_secs(5)).unwrap();
        assert!(r.ok && r.msg.as_deref() == Some("pong"));
    }
}
