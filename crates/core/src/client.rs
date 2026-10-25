//! Клиентская сторона: гарантировать, что демон запущен, и отправить ему запрос.

use crate::{
    config::Config,
    ipc,
    ipc::ClientError,
    paths,
    protocol::{Request, Response},
};
use std::{
    thread,
    time::{Duration, Instant},
};

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Запускает демон из установленной копии (путь берётся из config.json). Не ждёт готовности.
///
/// `CreateProcess` без наследования хэндлов (`bInheritHandles = FALSE`): через `Command` демон унаследовал бы
/// трубу вывода родителя (`discordctl setup | …`), и труба не закрывалась бы, пока жив демон.
pub fn spawn_daemon() -> Result<(), String> {
    use std::{mem, ptr};
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW},
    };
    let exe = Config::load().daemon_exe.ok_or("daemon is not installed (no daemon_exe in config)")?;
    if !exe.is_file() {
        return Err(format!("daemon binary not found: {}", exe.display()));
    }
    let mut cmd: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut si: STARTUPINFOW = mem::zeroed();
        si.cb = mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = mem::zeroed();
        let ok = CreateProcessW(
            ptr::null(),
            cmd.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW,
            ptr::null(),
            ptr::null(),
            &si,
            &mut pi,
        );
        if ok == 0 {
            return Err(format!("cannot start daemon: {}", std::io::Error::last_os_error()));
        }
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    }
    Ok(())
}

/// Проверяет демон, при необходимости запускает и ждёт до `timeout`.
pub fn ensure_daemon(timeout: Duration) -> Result<(), String> {
    let pipe = paths::pipe_name();
    match ipc::request(&pipe, &Request::Ping, Duration::from_secs(2)) {
        Ok(_) => return Ok(()),
        Err(ClientError::NotRunning) => {}
        Err(e) => return Err(e.to_string()),
    }
    spawn_daemon()?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
        if ipc::request(&pipe, &Request::Ping, Duration::from_secs(1)).is_ok() {
            return Ok(());
        }
    }
    Err("daemon did not become ready in time".into())
}

/// Запрос к демону, запуская его при необходимости.
pub fn call(req: &Request, timeout: Duration) -> Result<Response, String> {
    ensure_daemon(timeout)?;
    ipc::request(&paths::pipe_name(), req, timeout).map_err(|e| e.to_string())
}
