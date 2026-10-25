//! `discordctl doctor`: проверка установки и подсказки, что делать. `--fix` чинит то, что безопасно чинить автоматически.

use anyhow::Result;
use discordctl_core::{
    config::Config,
    handler, paths,
    protocol::Request,
    shim::{self, State, Target},
};
use serde_json::{json, Value};

use crate::{
    api::{ask, payload_call, sibling},
    ops, ui, winsys,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Fail,
}

impl Level {
    fn word(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Fail => "fail",
        }
    }
}

pub struct Check {
    pub name: &'static str,
    pub level: Level,
    pub detail: String,
    pub hint: Option<String>,
}

fn c(name: &'static str, level: Level, detail: impl Into<String>, hint: Option<&str>) -> Check {
    Check { name, level, detail: detail.into(), hint: hint.map(str::to_owned) }
}

const AV_HINT: &str = "If an antivirus (Kaspersky avp.exe is running here) blocked or quarantined a file, restore it from quarantine, \
add the folder %LOCALAPPDATA%\\discordctl and the folder with discordctl.exe to its trusted/excluded list \
(Kaspersky: Settings > Protection > Threats and exclusions > Manage exclusions), then run `discordctl doctor --fix`. \
The files are unsigned and hook into another program's launch, which antivirus heuristics dislike; you can also send them to the vendor as a false positive.";

/// Все проверки. `fix` чинит безопасное по ходу дела; печатью занимается вызывающий.
pub fn collect(fix: bool) -> Result<(Vec<Check>, Vec<String>)> {
    let cfg = Config::load();
    let root = cfg.root();
    let mut checks: Vec<Check> = vec![];
    let mut fixed: Vec<String> = vec![];

    // --- установка Discord
    match paths::latest_app_dir(&root) {
        Some(dir) => checks.push(c(
            "Discord install",
            Level::Ok,
            format!("{} ({})", dir.file_name().unwrap().to_string_lossy(), root.display()),
            None,
        )),
        None => checks.push(c(
            "Discord install",
            Level::Fail,
            format!("no app-* folder with Discord.exe in {}", root.display()),
            Some("Install Discord, or point discordctl at it: `discordctl install --root <folder>`."),
        )),
    }

    // --- файлы discordctl в bin/
    let bin = paths::bin_dir();
    let mut missing: Vec<&str> = ops::BIN_FILES.iter().copied().filter(|n| !bin.join(n).is_file()).collect();
    missing.retain(|n| *n != "discordctl-mcp.exe"); // MCP-сервер необязателен
    if !missing.is_empty() && fix && cfg.shim_installed {
        let mut restored = vec![];
        for n in &missing {
            if let Ok(src) = sibling(n) {
                let _ = std::fs::create_dir_all(&bin);
                if std::fs::copy(src, bin.join(n)).is_ok() {
                    restored.push(n.to_string());
                }
            }
        }
        if !restored.is_empty() {
            fixed.push(format!("restored {} from the build folder", restored.join(", ")));
        }
        missing.retain(|n| !bin.join(n).is_file());
    }
    if cfg.shim_installed && !missing.is_empty() {
        checks.push(c(
            "installed binaries",
            Level::Fail,
            format!("missing in {}: {}", bin.display(), missing.join(", ")),
            Some(AV_HINT),
        ));
    } else if cfg.shim_installed {
        // Копия в bin/ старее сборки рядом с CLI: частая причина «странного» поведения после пересборки.
        let stale: Vec<&str> = ops::BIN_FILES
            .iter()
            .copied()
            .filter(|n| {
                match (sibling(n).ok().and_then(|p| paths::sha256_file(&p)), paths::sha256_file(&bin.join(n))) {
                    (Some(a), Some(b)) => a != b,
                    _ => false,
                }
            })
            .collect();
        if stale.is_empty() {
            checks.push(c("installed binaries", Level::Ok, bin.display().to_string(), None));
        } else {
            checks.push(c("installed binaries", Level::Warn, format!("{} differ from this build", stale.join(", ")), Some("Run `discordctl install` (close Discord first, or stop the daemon with `discordctl daemon stop --force`).")));
        }
    } else {
        checks.push(c("installed binaries", Level::Info, "not installed", Some("Run `discordctl install`.")));
    }

    // --- шим
    for (name, t) in [("Update.exe shim", Target::Update), ("Discord.exe stub shim", Target::Stub)] {
        let mut st = shim::state(&root, t);
        if fix && cfg.shim_installed && st == State::NeedsRepair {
            let src = bin.join("discordctl-shim.exe");
            if matches!(shim::repair(&root, t, &src), Ok(true)) {
                fixed.push(format!("{} repaired", t.file()));
                st = shim::state(&root, t);
            }
        }
        let level = match (st, cfg.shim_installed, t) {
            (State::Installed, _, _) => Level::Ok,
            (State::Missing, _, Target::Stub) => Level::Info,
            (State::Missing, _, _) => Level::Fail,
            (State::NeedsRepair, _, _) => Level::Warn,
            (_, true, _) => Level::Fail,
            (_, false, _) => Level::Info,
        };
        let hint = match level {
            Level::Warn => Some("The updater overwrote the shim. The daemon repairs it by itself; otherwise `discordctl doctor --fix`."),
            Level::Fail => Some("Run `discordctl doctor --fix`, or `discordctl install` again. A missing Update.exe means Squirrel files were removed (often by an antivirus)."),
            Level::Info if st == State::NotInstalled => Some("Run `discordctl install`."),
            _ => None,
        };
        checks.push(c(name, level, format!("{st:?}"), hint));
    }
    if cfg.shim_installed {
        let real: Vec<&str> =
            ["Update.real.exe", "Discord.real.exe"].into_iter().filter(|f| !root.join(f).is_file()).collect();
        if !real.is_empty() && shim::state(&root, Target::Stub) != State::Missing
            || !root.join("Update.real.exe").is_file()
        {
            checks.push(c("original files", Level::Fail, format!("missing: {}", real.join(", ")), Some("Discord could not start or update. Restore the originals from the Discord installer, or run `discordctl uninstall`.")));
        } else {
            checks.push(c("original files", Level::Ok, "Update.real.exe present", None));
        }
    }

    // --- обработчик ссылок
    let want = handler::desired_command(&root);
    let mut cur = handler::read_command().ok().flatten();
    if fix && cfg.handler_hook {
        if let Ok(handler::Applied::Written { .. }) = handler::apply(&cfg) {
            fixed.push("discord:// handler pointed at the stub".into());
            cur = handler::read_command().ok().flatten();
        }
    }
    if !cfg.handler_hook {
        checks.push(c(
            "discord:// links",
            Level::Info,
            "hook is off: links open Discord without discordctl when it is closed",
            Some("`discordctl install` turns it on."),
        ));
    } else if cur.as_deref().is_some_and(|x| handler::same(x, &want)) {
        checks.push(c("discord:// links", Level::Ok, "handler points at the stub", None));
    } else {
        checks.push(c(
            "discord:// links",
            Level::Warn,
            format!("handler is {:?}", cur.unwrap_or_default()),
            Some("Discord rewrote it; the daemon restores it. Without a running daemon: `discordctl doctor --fix`."),
        ));
    }

    // --- автозапуск
    match winsys::run_key_discord() {
        Some(v) if v.to_lowercase().contains("update.exe") => checks.push(c("autostart", Level::Ok, "Run key starts Discord through Update.exe (the shim)", None)),
        Some(v) => checks.push(c("autostart", Level::Warn, format!("Run key is {v}"), Some("Autostart bypasses discordctl. Re-enable 'Open Discord' in Discord's settings, or it starts without the pipe."))),
        None => checks.push(c("autostart", Level::Info, "Discord is not set to start with Windows", None)),
    }

    // --- демон и хук
    let status = ask(&Request::Status).ok().flatten().and_then(|r| r.data);
    match &status {
        None => {
            checks.push(c(
                "daemon",
                Level::Info,
                "not running",
                Some("It starts with the first Discord launch (or `discordctl daemon start`)."),
            ));
            if ops::discord_running(&root) && cfg.shim_installed {
                checks.push(c(
                    "Discord process",
                    Level::Warn,
                    "Discord is running but the daemon is not: it was started without discordctl",
                    Some("Close Discord completely (also from the tray) and start it again."),
                ));
            }
        }
        Some(d) => {
            let running_sha = d["exe_sha256"].as_str().map(str::to_owned);
            let installed_sha = paths::sha256_file(&bin.join("discordctl-daemon.exe"));
            if running_sha.is_some() && installed_sha.is_some() && running_sha != installed_sha {
                checks.push(c("daemon", Level::Warn, format!("running v{} is older than the installed copy", d["version"].as_str().unwrap_or("?")), Some("Close Discord and start it again to load the new daemon (`discordctl daemon stop --force` closes Discord for you).")));
            } else {
                checks.push(c(
                    "daemon",
                    Level::Ok,
                    format!("running v{}, pid {}", d["version"].as_str().unwrap_or("?"), d["daemon_pid"]),
                    None,
                ));
            }
            if d["managed_discord"].is_null() {
                let lvl = if ops::discord_running(&root) { Level::Warn } else { Level::Info };
                let hint = (lvl == Level::Warn).then_some(
                    "Discord was started outside discordctl (no CDP pipe). Close it completely and start it again.",
                );
                checks.push(c("managed Discord", lvl, "not managed by discordctl", hint));
            } else {
                checks.push(c("managed Discord", Level::Ok, format!("pid {}", d["managed_discord"]["pid"]), None));
            }
            let hook = &d["hook"];
            let phase = hook["phase"].as_str().unwrap_or("?");
            match phase {
                "active" => {
                    checks.push(c("hook", Level::Ok, format!("active, {} stores", hook["payload"]["stores"]), None));
                    match payload_call("selfcheck", json!({})) {
                        Ok(v) if v["ok"] == true => checks.push(c(
                            "client compatibility",
                            Level::Ok,
                            format!("self-check passed (build {}, {})", v["build_number"].as_str().unwrap_or("?").chars().take(8).collect::<String>(), v["release_channel"].as_str().unwrap_or("?")),
                            None,
                        )),
                        Ok(v) => checks.push(c("client compatibility", Level::Fail, broken_parts(&v), Some("Discord changed internals. Reading may still work; see the list. This needs an update of the payload (payload/src), not a reinstall."))),
                        Err(e) => checks.push(c("client compatibility", Level::Warn, format!("self-check failed: {e}"), None)),
                    }
                }
                "idle" => checks.push(c("hook", Level::Info, "idle (no managed Discord)", None)),
                "waiting_page" | "installing" | "waiting_payload" => checks.push(c("hook", Level::Warn, phase.to_owned(), Some("Wait for Discord to finish loading and run doctor again."))),
                _ => checks.push(c("hook", Level::Fail, format!("{phase}: {}", hook["error"].as_str().unwrap_or("")), Some("`discordctl status` shows details; `discordctl hook reload` retries; check the daemon log in %LOCALAPPDATA%\\discordctl."))),
            }
        }
    }

    // --- приложения
    if status.is_some() {
        if let Ok(r) = ask(&Request::AuthList) {
            let n = r.and_then(|r| r.data).and_then(|d| d.as_array().map(Vec::len)).unwrap_or(0);
            checks.push(c(
                "apps",
                Level::Info,
                format!("{n} app(s) with tokens"),
                (n > 0).then_some("`discordctl auth list` shows their rights, `discordctl revoke <app>` removes one."),
            ));
        }
    }

    // --- антивирус и логи
    if winsys::process_running("avp.exe") {
        checks.push(c("antivirus", Level::Info, "Kaspersky (avp.exe) is running", Some(AV_HINT)));
    }
    for name in ["daemon", "shim"] {
        let p = paths::data_dir().join(format!("{name}.log"));
        if let Ok(m) = std::fs::metadata(&p) {
            if m.len() > 5 * 1024 * 1024 {
                checks.push(c(
                    "logs",
                    Level::Warn,
                    format!("{} is {} MB", p.display(), m.len() / 1024 / 1024),
                    Some("Delete it; logs rotate at 1 MB in current builds."),
                ));
            }
        }
    }

    Ok((checks, fixed))
}

pub fn run(json_out: bool, fix: bool) -> Result<bool> {
    let (checks, fixed) = collect(fix)?;
    let worst = checks.iter().map(|x| x.level).max().unwrap_or(Level::Ok);
    if json_out {
        let items: Vec<Value> = checks
            .iter()
            .map(|x| json!({ "name": x.name, "level": x.level.word(), "detail": x.detail, "hint": x.hint }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "ok": worst < Level::Fail, "checks": items, "fixed": fixed }))?
        );
    } else {
        println!();
        for x in &checks {
            let mark = match x.level {
                Level::Ok => console::style("✔").green().bold(),
                Level::Info => console::style("•").cyan(),
                Level::Warn => console::style("!").yellow().bold(),
                Level::Fail => console::style("✖").red().bold(),
            };
            println!("{mark} {:<22} {}", console::style(x.name).bold(), x.detail);
            if let (true, Some(h)) = (x.level >= Level::Warn || x.name == "antivirus", &x.hint) {
                ui::hint(h);
            }
        }
        for f in &fixed {
            ui::ok(format!("fixed: {f}"));
        }
        let (w, f) = (
            checks.iter().filter(|x| x.level == Level::Warn).count(),
            checks.iter().filter(|x| x.level == Level::Fail).count(),
        );
        println!();
        if f + w == 0 {
            ui::ok(format!("{} checks, everything is in order", checks.len()));
        } else {
            let line = format!("{} checks: {} problem(s), {} warning(s)", checks.len(), f, w);
            if f > 0 {
                ui::fail(line)
            } else {
                ui::warn(line)
            }
            if !fix {
                ui::hint("`discordctl doctor --fix` repairs what is safe to repair");
            }
        }
    }
    Ok(worst < Level::Fail)
}

/// Краткий список того, что пропало по самопроверке payload.
fn broken_parts(v: &Value) -> String {
    let mut parts = vec![];
    for (name, s) in v["stores"].as_object().into_iter().flatten() {
        if s["found"] == false {
            parts.push(format!("store {name} not found"));
        } else if let Some(m) = s["missing_methods"].as_array().filter(|m| !m.is_empty()) {
            parts.push(format!("{name} lacks {}", m.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")));
        }
    }
    for (name, ok) in v["actions"].as_object().into_iter().flatten() {
        if ok == false {
            parts.push(format!("actions {name} not found"));
        }
    }
    if parts.is_empty() {
        "self-check reported a problem".into()
    } else {
        parts.join("; ")
    }
}
