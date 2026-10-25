//! Обработчик ссылок `discord://` в реестре (HKCU, права администратора не нужны).
//!
//! Discord при каждом запуске записывает туда путь к `app-X.Y.Z\Discord.exe`: ссылка, открытая при закрытом Discord,
//! запустила бы его без CDP-пайпа. Мы указываем на корневую заглушку `<root>\Discord.exe` (там стоит шим, путь не зависит
//! от версии), а демон следит за ключом и возвращает значение после каждой перезаписи.

use std::{io, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegNotifyChangeKeyValue, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, KEY_WRITE, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    },
};

/// Корневой ключ протокола. Для тестов подменяется `DISCORDCTL_HANDLER_KEY`, чтобы не трогать настоящий реестр.
pub fn key_path() -> String {
    std::env::var("DISCORDCTL_HANDLER_KEY").unwrap_or_else(|_| r"Software\Classes\discord".to_owned())
}

fn command_key() -> String {
    format!(r"{}\shell\open\command", key_path())
}

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

/// Команда открытия ссылки или `None`, если ключа нет.
pub fn read_command() -> io::Result<Option<String>> {
    unsafe {
        let mut key: HKEY = ptr::null_mut();
        let rc = RegOpenKeyExW(HKEY_CURRENT_USER, wide(&command_key()).as_ptr(), 0, KEY_READ, &mut key);
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(rc)?;
        let mut kind = 0u32;
        let mut len = 0u32;
        let rc = RegQueryValueExW(key, ptr::null(), ptr::null(), &mut kind, ptr::null_mut(), &mut len);
        if rc == ERROR_FILE_NOT_FOUND {
            RegCloseKey(key);
            return Ok(None);
        }
        let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
        let rc2 = if rc == ERROR_SUCCESS {
            RegQueryValueExW(key, ptr::null(), ptr::null(), &mut kind, buf.as_mut_ptr() as *mut u8, &mut len)
        } else {
            rc
        };
        RegCloseKey(key);
        check(rc2)?;
        let n = (len as usize / 2).min(buf.len());
        let text = String::from_utf16_lossy(&buf[..n]);
        Ok(Some(text.trim_end_matches('\0').to_owned()))
    }
}

/// Записывает команду открытия ссылки, создавая ключи при необходимости. Заодно помечает ключ как URL-протокол.
pub fn write_command(command: &str) -> io::Result<()> {
    unsafe {
        let mut key: HKEY = ptr::null_mut();
        check(RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(&command_key()).as_ptr(),
            0,
            ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        ))?;
        let data = wide(command);
        let rc = RegSetValueExW(key, ptr::null(), 0, REG_SZ, data.as_ptr() as *const u8, (data.len() * 2) as u32);
        RegCloseKey(key);
        check(rc)
    }
}

/// Ждёт изменения под ключом протокола (включая вложенные). Блокирует поток; ключа может не быть — тогда ждёт родителя.
pub fn wait_change() -> io::Result<()> {
    unsafe {
        let mut key: HKEY = ptr::null_mut();
        check(RegOpenKeyExW(HKEY_CURRENT_USER, wide(&key_path()).as_ptr(), 0, KEY_NOTIFY, &mut key))?;
        let rc =
            RegNotifyChangeKeyValue(key, 1, REG_NOTIFY_CHANGE_LAST_SET | REG_NOTIFY_CHANGE_NAME, ptr::null_mut(), 0);
        RegCloseKey(key);
        check(rc)
    }
}

/// Команда, которую хотим видеть: заглушка в корне установки.
pub fn desired_command(root: &Path) -> String {
    format!("\"{}\" --url -- \"%1\"", root.join("Discord.exe").display())
}

/// Результат `apply`.
#[derive(Debug, PartialEq)]
pub enum Applied {
    /// Хук выключен в настройках или нет корневой заглушки (ссылку открыть нечем).
    Skipped,
    AlreadyOurs,
    /// Записали свою команду; `previous` — что стояло (его запоминаем в настройках для отката).
    Written {
        previous: Option<String>,
    },
}

/// Приводит обработчик к нужному виду, если хук включён и заглушка на месте. Идемпотентно.
pub fn apply(cfg: &crate::config::Config) -> io::Result<Applied> {
    let root = cfg.root();
    if !cfg.handler_hook || crate::shim::state(&root, crate::shim::Target::Stub) != crate::shim::State::Installed {
        return Ok(Applied::Skipped);
    }
    let want = desired_command(&root);
    let current = read_command()?;
    if current.as_deref().is_some_and(|c| same(c, &want)) {
        return Ok(Applied::AlreadyOurs);
    }
    write_command(&want)?;
    Ok(Applied::Written { previous: current })
}

pub fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_points_at_the_root_stub() {
        let c = desired_command(Path::new(r"C:\Users\x\AppData\Local\Discord"));
        assert_eq!(c, r#""C:\Users\x\AppData\Local\Discord\Discord.exe" --url -- "%1""#);
        assert!(same(&c, &c.to_uppercase()), "Windows paths compare case-insensitively");
    }

    /// Работает с отдельным ключом, настоящий `discord://` не трогает.
    #[test]
    fn registry_round_trip() {
        std::env::set_var("DISCORDCTL_HANDLER_KEY", r"Software\discordctl-test\handler-roundtrip");
        assert_eq!(read_command().unwrap(), None);
        write_command(r#""C:\a b\Discord.exe" --url -- "%1""#).unwrap();
        assert_eq!(read_command().unwrap().as_deref(), Some(r#""C:\a b\Discord.exe" --url -- "%1""#));
        write_command("second").unwrap();
        assert_eq!(read_command().unwrap().as_deref(), Some("second"));
        // Уборка тестового ключа.
        let _ = std::process::Command::new("reg").args(["delete", r"HKCU\Software\discordctl-test", "/f"]).output();
    }
}
