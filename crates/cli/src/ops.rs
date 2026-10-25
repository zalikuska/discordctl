//! Установка и удаление: файлы, шим, настройки, PATH, перезапуск Discord.

use crate::api::{ask, shim_status_value, sibling, stop_daemon};
use anyhow::{bail, Context, Result};
use discordctl_core::{
    config::Config,
    handler, paths,
    protocol::Request,
    shim::{self, Target},
};
use serde_json::{json, Value};
use std::{path::PathBuf, process::Command, time::Duration};

/// Файлы, которые кладём в `bin/`. Остальные нужны всегда; MCP-сервера в сборке может не быть.
pub const BIN_FILES: [&str; 4] =
    ["discordctl.exe", "discordctl-shim.exe", "discordctl-daemon.exe", "discordctl-mcp.exe"];

fn optional(name: &str) -> bool {
    name == "discordctl-mcp.exe"
}

/// Копирует собранные файлы в `bin/`. Работающий демон держит свой exe, поэтому вызывающий сначала останавливает его.
/// Если CLI запущен уже из `bin/`, сам себя не копирует.
pub fn copy_binaries() -> Result<PathBuf> {
    let bin = paths::bin_dir();
    std::fs::create_dir_all(&bin)?;
    for name in BIN_FILES {
        match sibling(name) {
            Ok(src) => {
                let dst = bin.join(name);
                let same = std::fs::canonicalize(&src).ok() == std::fs::canonicalize(&dst).ok() && dst.exists();
                if !same {
                    std::fs::copy(&src, &dst).with_context(|| format!("copying {name}"))?;
                }
            }
            Err(_) if optional(name) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(bin)
}

/// Подменяет `Update.exe` и корневую заглушку шимом; при сбое возвращает уже сделанное.
fn place_shims(root: &std::path::Path, shim_copy: &std::path::Path) -> Result<bool> {
    let mut placed: Vec<Target> = vec![];
    for t in shim::TARGETS {
        if t == Target::Stub && shim::state(root, t) == shim::State::Missing {
            continue; // в этой установке нет корневой заглушки
        }
        match shim::place(root, t, shim_copy) {
            Ok(true) => placed.push(t),
            Ok(false) => {}
            Err(e) => {
                for done in placed.iter().rev() {
                    let _ = shim::remove(root, *done);
                }
                return Err(e).with_context(|| format!("installing shim on {} (earlier steps rolled back)", t.file()));
            }
        }
    }
    Ok(!placed.is_empty())
}

/// Полная установка: файлы, шим на обе цели, настройки и (по желанию) хук ссылок `discord://`.
/// `link_hook`: `Some(true/false)` задаёт хук, `None` оставляет как было. `progress` получает названия шагов.
pub fn install_with(root: PathBuf, link_hook: Option<bool>, progress: &mut dyn FnMut(&str)) -> Result<Value> {
    if shim::state(&root, Target::Update) == shim::State::Missing {
        bail!(
            "{} not found: is Discord installed there? (use --root for a custom folder)",
            Target::Update.path(&root).display()
        );
    }
    progress("stopping the old daemon");
    stop_daemon(false)
        .context("cannot stop the running daemon (close Discord first, or run `discordctl daemon stop --force`)")?;
    progress("copying files");
    let bin = copy_binaries()?;
    progress("installing the launch shim");
    let changed = place_shims(&root, &bin.join("discordctl-shim.exe"))?;

    let mut cfg = Config::load();
    cfg.discord_root = Some(root.clone());
    cfg.daemon_exe = Some(bin.join("discordctl-daemon.exe"));
    cfg.shim_installed = true;
    if let Some(h) = link_hook {
        cfg.handler_hook = h;
    }
    cfg.save()?;

    let mut link = "off".to_owned();
    if cfg.handler_hook {
        progress("pointing discord:// links at the shim");
        match handler::apply(&cfg) {
            Ok(handler::Applied::Written { previous }) => {
                if cfg.handler_original.is_none() {
                    cfg.handler_original = previous;
                    cfg.save()?;
                }
                link = "installed".into();
            }
            Ok(handler::Applied::AlreadyOurs) => link = "already installed".into(),
            Ok(handler::Applied::Skipped) => link = "skipped (no root Discord.exe stub in this install)".into(),
            Err(e) => link = format!("failed: {e}"),
        }
    }
    Ok(json!({ "changed": changed, "bin": bin, "link_handler": link, "shim": shim_status_value(&root) }))
}

pub fn install(root: PathBuf, link_hook: Option<bool>) -> Result<Value> {
    install_with(root, link_hook, &mut |_| {})
}

/// Возвращает установку Discord как была. Работающий управляемый Discord не закрывает: демон выйдет сам, когда его закроют.
pub fn uninstall(purge: bool) -> Result<Value> {
    let mut cfg = Config::load();
    let root = cfg.root();

    // Очистку проверяем до любых изменений: демон держит файлы, из которых запущен.
    let status = ask(&Request::Status)?.and_then(|r| r.data);
    let managed = status.as_ref().is_some_and(|d| !d["managed_discord"].is_null());
    if purge {
        if managed {
            bail!("--purge needs Discord to be closed first: the running daemon holds the files it would delete");
        }
        crate::approve::require_terminal()?;
        let dir = paths::data_dir();
        println!("--purge will delete {} including the app tokens (apps.json) and the audit log.", dir.display());
        if !crate::approve::ask("Delete it?", false)? {
            bail!("cancelled");
        }
    }

    // Сначала выключаем слежение, потом возвращаем файлы: иначе демон увидит наши же изменения и поставит шим обратно.
    let was = (cfg.shim_installed, cfg.handler_hook);
    cfg.shim_installed = false;
    cfg.handler_hook = false;
    cfg.save()?;

    let mut changed = false;
    let mut errors = vec![];
    for t in shim::TARGETS {
        match shim::remove(&root, t) {
            Ok(c) => changed |= c,
            Err(e) => errors.push(format!("{}: {e:#}", t.file())),
        }
    }
    if !errors.is_empty() {
        // Слежение возвращаем: часть шима осталась на месте, и демон должен её охранять.
        cfg.shim_installed = was.0;
        cfg.handler_hook = was.1;
        let _ = cfg.save();
        bail!("uninstall incomplete: {}", errors.join("; "));
    }
    // Обработчик ссылок после отката указывает на корневой Discord.exe, то есть снова на настоящую заглушку Squirrel:
    // она рабочая и не зависит от версии, поэтому значение не трогаем, а слежение уже выключено.

    let daemon = if status.is_none() {
        "not running"
    } else if managed {
        "still running: it exits by itself when you close Discord"
    } else {
        stop_daemon(false)?;
        "stopped"
    };

    let mut purged = false;
    if purge {
        std::thread::sleep(Duration::from_millis(700));
        let dir = paths::data_dir();
        let mut last = None;
        for _ in 0..20 {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {
                    purged = true;
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    purged = true;
                    break;
                }
                Err(e) => {
                    last = Some(e);
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
        if !purged {
            bail!(
                "settings were reverted, but {} could not be deleted: {}",
                dir.display(),
                last.map(|e| e.to_string()).unwrap_or_default()
            );
        }
        if path_managed() {
            let _ = remove_bin_from_path();
        }
    }
    Ok(
        json!({ "changed": changed, "daemon": daemon, "purged": purged, "data_dir": paths::data_dir(), "shim": shim_status_value(&root) }),
    )
}

// ---- PATH ---------------------------------------------------------------------------------------------------

fn powershell(script: &str) -> Result<String> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .context("running powershell")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn user_path() -> Result<String> {
    powershell("[Environment]::GetEnvironmentVariable('Path','User')")
}

/// PATH трогаем только в обычной установке: с пользовательским `DISCORDCTL_HOME` (тесты, портативный режим) это был бы мусор в настройках.
pub fn path_managed() -> bool {
    std::env::var_os("DISCORDCTL_HOME").is_none()
}

pub fn bin_on_path() -> bool {
    let bin = paths::bin_dir().to_string_lossy().trim_end_matches('\\').to_lowercase();
    user_path().is_ok_and(|p| p.split(';').any(|e| e.trim_end_matches('\\').to_lowercase() == bin))
}

/// Добавляет `bin/` в PATH пользователя (новые окна терминала увидят `discordctl`).
pub fn add_bin_to_path() -> Result<bool> {
    if bin_on_path() {
        return Ok(false);
    }
    let bin = paths::bin_dir().to_string_lossy().into_owned();
    let cur = user_path()?;
    let new = if cur.is_empty() { bin } else { format!("{};{bin}", cur.trim_end_matches(';')) };
    powershell(&format!("[Environment]::SetEnvironmentVariable('Path','{}','User')", new.replace('\'', "''")))?;
    Ok(true)
}

fn remove_bin_from_path() -> Result<()> {
    let bin = paths::bin_dir().to_string_lossy().trim_end_matches('\\').to_lowercase();
    let cur = user_path()?;
    let kept: Vec<&str> =
        cur.split(';').filter(|e| !e.is_empty() && e.trim_end_matches('\\').to_lowercase() != bin).collect();
    powershell(&format!(
        "[Environment]::SetEnvironmentVariable('Path','{}','User')",
        kept.join(";").replace('\'', "''")
    ))?;
    Ok(())
}

// ---- процесс Discord -----------------------------------------------------------------------------------------

/// Скрипт PowerShell над процессами Discord из этой установки (по пути к exe). Имя процесса не годится:
/// у человека может быть другая установка или копия, и закрывать чужое нельзя.
fn discord_script(root: &std::path::Path, action: &str) -> String {
    let root = root.to_string_lossy().replace('\'', "''");
    format!(
        "Get-Process -Name Discord -ErrorAction SilentlyContinue | Where-Object {{ $_.Path -and $_.Path.StartsWith('{root}', [StringComparison]::OrdinalIgnoreCase) }} | {action}"
    )
}

pub fn discord_running(root: &std::path::Path) -> bool {
    powershell(&discord_script(root, "Measure-Object | ForEach-Object { $_.Count }"))
        .ok()
        .and_then(|n| n.trim().parse::<u32>().ok())
        .is_some_and(|n| n > 0)
}

/// Закрывает процессы Discord этой установки (звонок при этом прервётся). Возвращает, закрылись ли они.
pub fn close_discord(root: &std::path::Path) -> bool {
    let _ = powershell(&discord_script(root, "Stop-Process -Force"));
    for _ in 0..20 {
        if !discord_running(root) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    !discord_running(root)
}
