//! Расположение файлов. Переменные окружения нужны для тестов на поддельной установке.

use std::{
    env,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn local_app_data() -> PathBuf {
    env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn data_dir() -> PathBuf {
    env::var_os("DISCORDCTL_HOME").map(PathBuf::from).unwrap_or_else(|| local_app_data().join("discordctl"))
}

pub fn bin_dir() -> PathBuf {
    data_dir().join("bin")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn default_discord_root() -> PathBuf {
    env::var_os("DISCORDCTL_DISCORD_ROOT").map(PathBuf::from).unwrap_or_else(|| local_app_data().join("Discord"))
}

/// Имя pipe демона. Привязано к пользователю, чтобы сессии разных пользователей не пересекались.
pub fn pipe_name() -> String {
    if let Ok(n) = env::var("DISCORDCTL_PIPE") {
        return n;
    }
    let user: String = env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!(r"\\.\pipe\discordctl-{user}")
}

/// Самая свежая папка `app-X.Y.Z`, в которой лежит Discord.exe.
pub fn latest_app_dir(root: &Path) -> Option<PathBuf> {
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(ver) = name.strip_prefix("app-") else { continue };
        let parts: Option<Vec<u64>> = ver.split('.').map(|p| p.parse().ok()).collect();
        let Some(parts) = parts else { continue };
        let dir = entry.path();
        if !dir.join("Discord.exe").is_file() {
            continue;
        }
        if best.as_ref().is_none_or(|(b, _)| parts > *b) {
            best = Some((parts, dir));
        }
    }
    best.map(|(_, d)| d)
}

/// Дописывает строку в `<data>/<name>.log`. Ошибки игнорируются: лог не должен ломать запуск.
pub fn log(name: &str, msg: &str) {
    let _ = std::fs::create_dir_all(data_dir());
    // Лог не растёт без конца: после 1 МБ прошлый файл заменяется (хранится одно поколение `.1.log`).
    let path = data_dir().join(format!("{name}.log"));
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = std::fs::rename(&path, data_dir().join(format!("{name}.1.log")));
    }
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(data_dir().join(format!("{name}.log"))) {
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

/// SHA-256 байтов в hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 файла в hex или `None`, если файла нет или он не читается.
pub fn sha256_file(path: &Path) -> Option<String> {
    Some(sha256_hex(&std::fs::read(path).ok()?))
}
