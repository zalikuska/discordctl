//! Системные вызовы Windows напрямую: процессы и реестр пользователя.
//!
//! Раньше для этого запускались `powershell -Command`, `tasklist` и `reg`. Неподписанный exe, который порождает
//! PowerShell с `Stop-Process -Force`, выглядит для эвристик антивируса как дроппер, а прямые вызовы API ничего
//! лишнего не запускают.

use std::{io, mem, path::PathBuf, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
        },
        Registry::{
            RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ,
        },
        Threading::{
            OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_TERMINATE,
        },
    },
    UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE},
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn check(code: u32) -> io::Result<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

// ---- процессы -----------------------------------------------------------------------------------------------

/// Пары (pid, имя exe) всех процессов.
fn processes() -> Vec<(u32, String)> {
    let mut out = vec![];
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut entry: PROCESSENTRY32W = mem::zeroed();
        entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut entry) != 0;
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            out.push((entry.th32ProcessID, String::from_utf16_lossy(&entry.szExeFile[..len])));
            more = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// Полный путь к exe процесса; `None`, если процесс чужой или уже завершился.
fn image_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// Идёт ли процесс с таким именем образа (без учёта регистра).
pub fn process_running(image: &str) -> bool {
    processes().iter().any(|(_, name)| name.eq_ignore_ascii_case(image))
}

/// Идентификаторы процессов `image`, чей exe лежит внутри `root`. Чужие копии с тем же именем не попадают.
pub fn pids_under(image: &str, root: &std::path::Path) -> Vec<u32> {
    // С разделителем на конце: `...\Discord` не должен совпасть с `...\DiscordPTB` или `...\Discord2`.
    let root = format!("{}\\", root.to_string_lossy().to_lowercase().trim_end_matches(['\\', '/']));
    processes()
        .into_iter()
        .filter(|(_, name)| name.eq_ignore_ascii_case(image))
        .filter(|(pid, _)| image_path(*pid).is_some_and(|p| p.to_string_lossy().to_lowercase().starts_with(&root)))
        .map(|(pid, _)| pid)
        .collect()
}

pub fn terminate(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h.is_null() {
            return false;
        }
        let ok = TerminateProcess(h, 1) != 0;
        CloseHandle(h);
        ok
    }
}

// ---- реестр пользователя ------------------------------------------------------------------------------------

/// Значение строкового параметра в `HKCU\<subkey>` и его тип (`REG_SZ` или `REG_EXPAND_SZ`); `None`, если нет.
pub fn read_user_string(subkey: &str, name: &str) -> io::Result<Option<(String, u32)>> {
    unsafe {
        let mut key: HKEY = ptr::null_mut();
        let rc = RegOpenKeyExW(HKEY_CURRENT_USER, wide(subkey).as_ptr(), 0, KEY_READ, &mut key);
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(rc)?;
        let name_w = wide(name);
        let mut kind = 0u32;
        let mut len = 0u32;
        let mut rc = RegQueryValueExW(key, name_w.as_ptr(), ptr::null(), &mut kind, ptr::null_mut(), &mut len);
        let mut buf = vec![];
        // Значение могло вырасти между двумя вызовами: повторяем с новым размером.
        for _ in 0..4 {
            if rc != ERROR_SUCCESS && rc != ERROR_MORE_DATA {
                break;
            }
            buf = vec![0u16; (len as usize).div_ceil(2) + 1];
            let mut cap = (buf.len() * 2) as u32;
            rc = RegQueryValueExW(key, name_w.as_ptr(), ptr::null(), &mut kind, buf.as_mut_ptr() as *mut u8, &mut cap);
            len = cap;
            if rc != ERROR_MORE_DATA {
                break;
            }
        }
        RegCloseKey(key);
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(rc)?;
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return Ok(None);
        }
        let n = (len as usize / 2).min(buf.len());
        Ok(Some((String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_owned(), kind)))
    }
}

/// Записывает строковый параметр в `HKCU\<subkey>`, создавая ключ при необходимости.
pub fn write_user_string(subkey: &str, name: &str, value: &str, kind: u32) -> io::Result<()> {
    unsafe {
        let mut key: HKEY = ptr::null_mut();
        check(RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(subkey).as_ptr(),
            0,
            ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        ))?;
        let data = wide(value);
        let rc = RegSetValueExW(key, wide(name).as_ptr(), 0, kind, data.as_ptr() as *const u8, (data.len() * 2) as u32);
        RegCloseKey(key);
        check(rc)
    }
}

/// Сообщает запущенным программам (проводнику), что переменные окружения изменились: новые окна терминала увидят PATH.
pub fn broadcast_env_change() {
    let env = wide("Environment");
    unsafe {
        let mut result = 0usize;
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            env.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            3000,
            &mut result,
        );
    }
}

const ENV_KEY: &str = "Environment";

/// PATH пользователя как есть (с `%VAR%`) и тип значения. Пусто, если параметра нет.
pub fn user_path() -> io::Result<(String, u32)> {
    Ok(read_user_string(ENV_KEY, "Path")?.unwrap_or_else(|| (String::new(), REG_EXPAND_SZ)))
}

pub fn set_user_path(value: &str, kind: u32) -> io::Result<()> {
    write_user_string(ENV_KEY, "Path", value, kind)?;
    broadcast_env_change();
    Ok(())
}

/// Команда автозапуска Discord из `HKCU\...\Run`.
pub fn run_key_discord() -> Option<String> {
    read_user_string(r"Software\Microsoft\Windows\CurrentVersion\Run", "Discord").ok().flatten().map(|(v, _)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_own_process() {
        let me = std::env::current_exe().unwrap();
        let image = me.file_name().unwrap().to_string_lossy().into_owned();
        assert!(process_running(&image));
        assert!(pids_under(&image, me.parent().unwrap()).contains(&std::process::id()));
        assert!(pids_under(&image, std::path::Path::new(r"Z:\no\such\dir")).is_empty());
        // Папка-«сосед» с тем же началом имени (Discord и DiscordPTB) не считается внутренней.
        let dir = me.parent().unwrap().to_string_lossy().into_owned();
        assert!(pids_under(&image, std::path::Path::new(&dir[..dir.len() - 1])).is_empty());
    }

    #[test]
    fn path_is_readable() {
        // Параметр Path пользователя обычно есть; главное, что чтение не падает.
        user_path().unwrap();
    }
}
