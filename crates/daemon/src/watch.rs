//! Слежение за корнем установки Discord.
//!
//! Squirrel при каждом запуске Discord восстанавливает оригинальную заглушку `Discord.exe`,
//! затирая шим. Периодической проверки раз в полминуты мало: в это окно ярлыки запускают
//! Discord мимо шима. Поэтому следим за папкой и чиним сразу после перезаписи.

use std::{mem, path::Path, ptr, thread, time::Duration};
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
        FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
        FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    },
};

const WATCHED: [&str; 2] = ["discord.exe", "update.exe"];

/// Имена файлов из буфера FILE_NOTIFY_INFORMATION.
fn changed_names(buf: &[u8], len: usize) -> Vec<String> {
    let mut names = vec![];
    let mut off = 0usize;
    while off + mem::size_of::<FILE_NOTIFY_INFORMATION>() <= len {
        // Читаем поля без предположений о выравнивании.
        let rd = |at: usize| u32::from_le_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
        let next = rd(off);
        let name_len = rd(off + 8);
        let start = off + 12;
        if start + name_len > len {
            break;
        }
        // Не `chunks_exact(2)`: его замена `as_chunks` появилась в Rust 1.88, а мы поддерживаем 1.82.
        let name = &buf[start..start + name_len];
        let units: Vec<u16> = (0..name_len / 2).map(|i| u16::from_le_bytes([name[2 * i], name[2 * i + 1]])).collect();
        names.push(String::from_utf16_lossy(&units).to_ascii_lowercase());
        if next == 0 {
            break;
        }
        off += next;
    }
    names
}

/// Блокирующий цикл: вызывает `on_change`, когда в корне изменился Discord.exe или Update.exe.
pub fn watch_root(root: &Path, on_change: impl Fn()) {
    let wide: Vec<u16> = root.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    loop {
        let h = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            thread::sleep(Duration::from_secs(5));
            continue;
        }
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut n = 0u32;
            let ok = unsafe {
                ReadDirectoryChangesW(
                    h,
                    buf.as_mut_ptr() as *mut _,
                    buf.len() as u32,
                    0,
                    FILE_NOTIFY_CHANGE_FILE_NAME
                        | FILE_NOTIFY_CHANGE_SIZE
                        | FILE_NOTIFY_CHANGE_LAST_WRITE
                        | FILE_NOTIFY_CHANGE_CREATION,
                    &mut n,
                    ptr::null_mut(),
                    None,
                )
            };
            if ok == 0 {
                break; // папку удалили или переустановили; открываем заново
            }
            // n == 0 значит переполнение буфера: событий слишком много, проверяем на всякий случай.
            if n == 0 || changed_names(&buf, n as usize).iter().any(|name| WATCHED.contains(&name.as_str())) {
                on_change();
            }
        }
        unsafe { CloseHandle(h) };
        thread::sleep(Duration::from_secs(2));
    }
}
