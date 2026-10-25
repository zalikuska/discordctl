//! Запуск процесса Discord с CDP-пайпом.
//!
//! Chromium на Windows берёт хэндлы из `--remote-debugging-io-pipes=<читать>,<писать>`.
//! Мы создаём два анонимных пайпа и отдаём ребёнку только их (handle list), поэтому
//! к CDP нет доступа ни у кого, кроме держателя наших концов.

use std::{
    ffi::c_void,
    fs::File,
    io, mem,
    os::windows::io::{FromRawHandle, RawHandle},
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT},
    Security::SECURITY_ATTRIBUTES,
    System::{
        Pipes::CreatePipe,
        Threading::{
            CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
            UpdateProcThreadAttribute, EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, STARTUPINFOEXW,
        },
    },
};

const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;

pub struct Spawned {
    pub pid: u32,
    /// Дескриптор процесса (владеет вызывающий, закрыть через CloseHandle).
    pub process: HANDLE,
    /// Наш конец «в браузер». Закрытие даёт Chromium EOF и завершает Discord, держать открытым.
    pub to_browser: File,
    /// Наш конец «из браузера». Его нужно читать, иначе Chromium упрётся в полный буфер.
    pub from_browser: File,
}

unsafe impl Send for Spawned {}

fn pipe() -> io::Result<(HANDLE, HANDLE)> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    let (mut r, mut w): (HANDLE, HANDLE) = (ptr::null_mut(), ptr::null_mut());
    if unsafe { CreatePipe(&mut r, &mut w, &sa, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((r, w))
}

fn quote(s: &str) -> String {
    if !s.is_empty() && !s.contains([' ', '\t', '"']) {
        s.to_owned()
    } else {
        format!("\"{}\"", s.replace('"', "\\\""))
    }
}

pub fn spawn_with_cdp(exe: &Path, cwd: &Path, extra: &[String]) -> io::Result<Spawned> {
    // Браузер читает r1 (мы пишем в w1) и пишет в w2 (мы читаем из r2).
    let (r1, w1) = pipe()?;
    let (r2, w2) = pipe()?;
    unsafe {
        SetHandleInformation(w1, HANDLE_FLAG_INHERIT, 0);
        SetHandleInformation(r2, HANDLE_FLAG_INHERIT, 0);
    }

    let mut cmd = format!(
        "\"{}\" --remote-debugging-pipe --remote-debugging-io-pipes={},{}",
        exe.display(),
        r1 as usize,
        w2 as usize
    );
    for a in extra {
        cmd.push(' ');
        cmd.push_str(&quote(a));
    }
    let mut cmd_w: Vec<u16> = cmd.encode_utf16().chain(Some(0)).collect();
    let cwd_w: Vec<u16> = cwd.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();

    let close_all = || unsafe {
        for h in [r1, w1, r2, w2] {
            CloseHandle(h);
        }
    };

    unsafe {
        let mut size = 0usize;
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
        let mut buf = vec![0u64; size.div_ceil(8)];
        let attrs = buf.as_mut_ptr() as *mut c_void;
        if InitializeProcThreadAttributeList(attrs, 1, 0, &mut size) == 0 {
            let e = io::Error::last_os_error();
            close_all();
            return Err(e);
        }
        let list: [HANDLE; 2] = [r1, w2];
        if UpdateProcThreadAttribute(
            attrs,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            list.as_ptr() as *const c_void,
            mem::size_of_val(&list),
            ptr::null_mut(),
            ptr::null(),
        ) == 0
        {
            let e = io::Error::last_os_error();
            DeleteProcThreadAttributeList(attrs);
            close_all();
            return Err(e);
        }

        let mut si: STARTUPINFOEXW = mem::zeroed();
        si.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
        si.lpAttributeList = attrs;
        let mut pi: PROCESS_INFORMATION = mem::zeroed();
        let ok = CreateProcessW(
            ptr::null(),
            cmd_w.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT,
            ptr::null(),
            cwd_w.as_ptr(),
            &si.StartupInfo,
            &mut pi,
        );
        let err = io::Error::last_os_error();
        DeleteProcThreadAttributeList(attrs);
        if ok == 0 {
            close_all();
            return Err(err);
        }
        CloseHandle(pi.hThread);
        // Концы ребёнка нам больше не нужны.
        CloseHandle(r1);
        CloseHandle(w2);
        Ok(Spawned {
            pid: pi.dwProcessId,
            process: pi.hProcess,
            to_browser: File::from_raw_handle(w1 as RawHandle),
            from_browser: File::from_raw_handle(r2 as RawHandle),
        })
    }
}
