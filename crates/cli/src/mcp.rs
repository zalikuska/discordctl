//! Подключение ИИ-агентов: приложение с токеном в демоне + запись MCP-сервера в настройки агента.

use crate::{
    api::{ask, owner_call},
    approve,
    cli::{AgentKind, Preset},
    ui,
};
use anyhow::{bail, Context, Result};
use discordctl_core::{config::Config, paths, permissions::Permissions, protocol::Request};
use serde_json::{json, Map, Value};
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

/// Имя сервера в настройках агента.
const SERVER: &str = "discord";

pub fn mcp_exe() -> PathBuf {
    paths::bin_dir().join("discordctl-mcp.exe")
}

/// Какие права выдаёт набор. Подтверждение каждой записи включено, кроме набора `Autonomous`: он отключает их все.
pub fn preset_permissions(p: Preset) -> Permissions {
    let mut perms = Permissions {
        list: ["servers", "channels", "chats", "friends", "me", "members", "voices"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        read: vec!["*".into()],
        ..Default::default()
    };
    if p != Preset::ReadOnly {
        perms.write = vec!["*".into()];
    }
    if p == Preset::Autonomous {
        // Всё, что умеет MCP-сервер, и без вопросов к человеку. Лимит записи в минуту в payload остаётся.
        perms.list.push("presence".into());
        perms.voice = vec!["*".into()];
        perms.voice_channels = vec!["*".into()];
        perms.stream = vec!["*".into()];
        perms.confirm = Some(vec![]);
    }
    perms.normalize();
    perms
}

fn app_name(a: AgentKind) -> &'static str {
    match a {
        AgentKind::Print => "mcp-agent",
        other => other.slug(),
    }
}

// ---- обнаружение ---------------------------------------------------------------------------------------------

fn appdata() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default()
}
fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default()
}

/// Папки настроек Claude Desktop. Версия из Microsoft Store (MSIX) хранит их не в `%APPDATA%\Claude`,
/// а внутри каталога пакета; приложение читает именно их, поэтому они идут первыми.
fn claude_desktop_dirs() -> Vec<PathBuf> {
    claude_desktop_dirs_in(&paths::local_app_data(), &appdata())
}

/// Базовые папки передаются параметрами, чтобы тесты не трогали переменные окружения всего процесса.
fn claude_desktop_dirs_in(local: &Path, roaming: &Path) -> Vec<PathBuf> {
    let mut dirs = store_dirs_in(local);
    let plain = roaming.join("Claude");
    if plain.is_dir() {
        dirs.push(plain);
    }
    dirs
}

fn store_dirs_in(local: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Ok(packages) = std::fs::read_dir(local.join("Packages")) {
        for e in packages.flatten() {
            if e.file_name().to_string_lossy().starts_with("Claude_") {
                let d = e.path().join("LocalCache").join("Roaming").join("Claude");
                if d.is_dir() {
                    dirs.push(d);
                }
            }
        }
    }
    dirs
}

/// Claude Desktop установлен: пакет из Store или обычный установщик (`%LOCALAPPDATA%\AnthropicClaude`).
/// Одной папки `%APPDATA%\Claude` мало: деинсталлятор оставляет её после удаления приложения.
fn claude_desktop_installed_in(local: &Path, roaming: &Path) -> bool {
    !store_dirs_in(local).is_empty() || (roaming.join("Claude").is_dir() && local.join("AnthropicClaude").is_dir())
}

fn config_path(a: AgentKind) -> Option<PathBuf> {
    match a {
        AgentKind::ClaudeDesktop => {
            let dir = claude_desktop_dirs().into_iter().next().unwrap_or_else(|| appdata().join("Claude"));
            Some(dir.join("claude_desktop_config.json"))
        }
        AgentKind::Cursor => Some(home().join(".cursor").join("mcp.json")),
        _ => None,
    }
}

/// Путь к команде `claude`: нативный `.exe` или `.cmd` из npm.
fn claude_cli() -> Option<PathBuf> {
    let out = Command::new("where").arg("claude").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let found: Vec<PathBuf> = text.lines().map(|l| PathBuf::from(l.trim())).filter(|p| p.is_file()).collect();
    found.iter().find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))).or(found.first()).cloned()
}

pub fn detected(a: AgentKind) -> bool {
    match a {
        AgentKind::ClaudeCode => claude_cli().is_some(),
        AgentKind::ClaudeDesktop => claude_desktop_installed_in(&paths::local_app_data(), &appdata()),
        AgentKind::Cursor => home().join(".cursor").is_dir(),
        AgentKind::Print => true,
    }
}

/// Агенты, которые есть на этом ПК, и всегда «показать настройки». Остальные не предлагаем:
/// нечего подключать (можно явно: `--agent claude-desktop`).
pub fn available() -> Vec<AgentKind> {
    ALL.iter().copied().filter(|a| detected(*a)).collect()
}

pub const ALL: [AgentKind; 4] = [AgentKind::ClaudeCode, AgentKind::ClaudeDesktop, AgentKind::Cursor, AgentKind::Print];

// ---- запуск claude -------------------------------------------------------------------------------------------

fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        arg.to_owned()
    } else {
        format!("\"{}\"", arg.replace('"', "\\\""))
    }
}

/// Запускает `claude …`. `.cmd`-обёртки npm запускаются через `cmd /C call`, аргументы квотируются вручную.
fn run_claude(args: &[&str]) -> Result<(bool, String)> {
    let exe =
        claude_cli().context("the `claude` command was not found (install Claude Code, or choose another agent)")?;
    let is_cmd = exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    let mut c;
    if is_cmd {
        c = Command::new("cmd");
        let line = format!(
            "call {} {}",
            quote(&exe.to_string_lossy()),
            args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
        );
        c.raw_arg(format!("/S /C \"{line}\""));
    } else {
        c = Command::new(&exe);
        c.args(args);
    }
    let out = c.output().context("running claude")?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text.trim().to_owned()))
}

// ---- JSON-настройки ------------------------------------------------------------------------------------------

fn read_json(path: &PathBuf) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(json!({})),
        Ok(t) => serde_json::from_str(&t).with_context(|| {
            format!("{} is not valid JSON: fix or move it, discordctl will not overwrite it", path.display())
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e).with_context(|| path.display().to_string()),
    }
}

fn write_json(path: &PathBuf, v: &Value) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Одна резервная копия прежнего файла: настройки агента не наши, терять их нельзя.
    let bak = path.with_extension("json.discordctl.bak");
    if path.is_file() && !bak.exists() {
        std::fs::copy(path, &bak)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(v)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn server_entry(token: &str) -> Value {
    json!({ "command": mcp_exe(), "args": [], "env": { "DISCORDCTL_TOKEN": token } })
}

fn json_register(path: &PathBuf, token: &str) -> Result<()> {
    let mut v = read_json(path)?;
    let root = v.as_object_mut().context("config root is not an object")?;
    let servers = root.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
    servers.as_object_mut().context("mcpServers is not an object")?.insert(SERVER.into(), server_entry(token));
    write_json(path, &v)
}

fn json_unregister(path: &PathBuf) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let mut v = read_json(path)?;
    let removed = v.get_mut("mcpServers").and_then(Value::as_object_mut).is_some_and(|m| m.remove(SERVER).is_some());
    if removed {
        write_json(path, &v)?;
    }
    Ok(removed)
}

fn json_has(path: &PathBuf) -> bool {
    read_json(path).ok().is_some_and(|v| v["mcpServers"][SERVER].is_object())
}

// ---- команды -------------------------------------------------------------------------------------------------

fn require_installed() -> Result<()> {
    if !Config::load().shim_installed || !mcp_exe().is_file() {
        bail!("discordctl is not installed yet (or the MCP server is missing): run `discordctl setup` first");
    }
    Ok(())
}

/// Выдаёт приложению права (после просмотра человеком) и возвращает токен.
fn grant(app: &str, description: &str, perms: &Permissions) -> Result<Option<String>> {
    match approve::review_compact(app, description, perms)? {
        Some(final_perms) => {
            let r = owner_call(&Request::AuthGrant { name: app.to_owned(), permissions: final_perms })?;
            Ok(r.data.and_then(|d| d["token"].as_str().map(str::to_owned)))
        }
        None => Ok(None),
    }
}

fn register(a: AgentKind, token: &str) -> Result<String> {
    match a {
        AgentKind::ClaudeCode => {
            let _ = run_claude(&["mcp", "remove", SERVER, "-s", "user"]); // старая запись, если была
            let exe = mcp_exe();
            let token_arg = format!("DISCORDCTL_TOKEN={token}");
            let (ok, text) =
                run_claude(&["mcp", "add", SERVER, "-s", "user", "-e", &token_arg, "--", &exe.to_string_lossy()])?;
            if !ok {
                bail!("`claude mcp add` failed: {text}");
            }
            Ok("registered with `claude mcp add` (user scope)".into())
        }
        AgentKind::ClaudeDesktop | AgentKind::Cursor => {
            let path = config_path(a).unwrap();
            json_register(&path, token)?;
            Ok(format!("added to {}", path.display()))
        }
        AgentKind::Print => {
            ui::say("\nPaste this into the agent's MCP settings:\n");
            println!("{}", serde_json::to_string_pretty(&json!({ "mcpServers": { SERVER: server_entry(token) } }))?);
            ui::say(format!(
                "\nor, for Claude Code:\n  claude mcp add {SERVER} -s user -e DISCORDCTL_TOKEN={token} -- \"{}\"",
                mcp_exe().display()
            ));
            Ok("printed above".into())
        }
    }
}

/// Подключение агента без вопросов и печати (экран мастера сам показал права и получил согласие).
/// Возвращает, как зарегистрировано, и текст настроек для агентов, которых мы не трогаем сами.
pub fn connect_quiet(a: AgentKind, perms: &Permissions) -> Result<(String, Option<String>)> {
    require_installed()?;
    let app = app_name(a);
    let r = owner_call(&Request::AuthGrant { name: app.to_owned(), permissions: perms.clone() })?;
    let token = r.data.and_then(|d| d["token"].as_str().map(str::to_owned)).context("the daemon returned no token")?;
    let result = match a {
        AgentKind::Print => Ok((
            "config to paste".to_owned(),
            Some(format!(
                "{}\n\nor, for Claude Code:\n  claude mcp add {SERVER} -s user -e DISCORDCTL_TOKEN={token} -- \"{}\"",
                serde_json::to_string_pretty(&json!({ "mcpServers": { SERVER: server_entry(&token) } }))?,
                mcp_exe().display()
            )),
        )),
        _ => register(a, &token).map(|how| (how, None)),
    };
    if result.is_err() {
        // Токен без записи в настройки никому не нужен: отзываем, чтобы не оставлять мёртвое приложение.
        let _ = owner_call(&Request::AuthRevoke { name: app.to_owned() });
    }
    result
}

pub fn install(mut agents: Vec<AgentKind>, preset: Preset, json_mode: bool) -> Result<()> {
    require_installed()?;
    if agents.is_empty() {
        let avail = available();
        let titles: Vec<String> = avail
            .iter()
            .map(|a| if *a == AgentKind::Print { a.title().to_owned() } else { format!("{} (found)", a.title()) })
            .collect();
        let items: Vec<&str> = titles.iter().map(String::as_str).collect();
        let none_found = avail == [AgentKind::Print];
        let defaults: Vec<bool> = avail.iter().map(|a| none_found || *a != AgentKind::Print).collect();
        let picked = ui::multiselect("Which agents should get Discord access?", &items, &defaults)?;
        agents = picked.into_iter().map(|i| avail[i]).collect();
        if agents.is_empty() {
            ui::info("nothing selected");
            return Ok(());
        }
    }
    let mut results = vec![];
    for a in agents {
        let app = app_name(a);
        if !detected(a) && a != AgentKind::Print {
            ui::warn(format!(
                "{} was not found on this PC; connecting anyway (the config is written where it would live)",
                a.title()
            ));
        }
        let perms = preset_permissions(preset);
        let Some(token) = grant(app, &format!("MCP server for {}", a.title()), &perms)? else {
            ui::fail(format!("{}: not granted", a.title()));
            results.push(json!({ "agent": a.slug(), "connected": false }));
            continue;
        };
        match register(a, &token) {
            Ok(how) => {
                ui::ok(format!("{}: {how}", ui::bold(a.title())));
                results.push(json!({ "agent": a.slug(), "app": app, "connected": true, "how": how }));
            }
            Err(e) => {
                // Токен без записи в настройки никому не нужен: отзываем, чтобы не оставлять мёртвое приложение.
                let _ = owner_call(&Request::AuthRevoke { name: app.to_owned() });
                ui::fail(format!("{}: {e:#}", a.title()));
                results.push(json!({ "agent": a.slug(), "connected": false, "error": format!("{e:#}") }));
            }
        }
    }
    if json_mode {
        println!("{}", serde_json::to_string_pretty(&json!({ "agents": results }))?);
        return Ok(());
    }
    if results.iter().any(|r| r["connected"] == true) {
        println!();
        ui::info("Restart the agent so it picks up the new server.");
        ui::info(format!(
            "The token sits in the agent's own config; it only carries the rights you approved. Revoke: {}",
            ui::bold("discordctl mcp remove --agent <name>")
        ));
        if preset == Preset::Assistant {
            ui::info(format!(
                "Writes wait for you: keep {} open in another window.",
                ui::bold("discordctl app approve")
            ));
        }
        if preset == Preset::Autonomous {
            ui::warn(format!(
                "Autonomous: the agent acts without asking you. Take it away any time: {}",
                ui::bold("discordctl mcp remove")
            ));
        }
    }
    Ok(())
}

pub fn status(json_mode: bool) -> Result<()> {
    let apps: Vec<String> = ask(&Request::AuthList)?
        .and_then(|r| r.data)
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|a| a["name"].as_str().map(str::to_owned))
        .collect();
    let mut rows = vec![];
    let mut values = vec![];
    for a in ALL.iter().filter(|a| **a != AgentKind::Print) {
        let configured = match a {
            AgentKind::ClaudeCode => {
                claude_cli().is_some() && run_claude(&["mcp", "get", SERVER]).is_ok_and(|(ok, _)| ok)
            }
            _ => config_path(*a).is_some_and(|p| json_has(&p)),
        };
        let has_app = apps.iter().any(|n| n == app_name(*a));
        values.push(json!({ "agent": a.slug(), "found": detected(*a), "configured": configured, "app": has_app }));
        let mark = |b: bool| if b { ui::green("yes") } else { ui::dim("no") };
        rows.push(vec![ui::bold(a.title()), mark(detected(*a)), mark(configured), mark(has_app)]);
    }
    if json_mode {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "agents": values, "mcp_server": mcp_exe(), "mcp_server_present": mcp_exe().is_file() })
            )?
        );
        return Ok(());
    }
    println!("{}", ui::table(&["AGENT", "FOUND", "CONNECTED", "TOKEN"], &rows));
    if !mcp_exe().is_file() {
        ui::warn("the MCP server is missing from the install folder: run `discordctl install`");
    }
    Ok(())
}

pub fn remove(agents: Vec<AgentKind>, json_mode: bool) -> Result<()> {
    let mut results = vec![];
    for a in agents {
        let mut removed_config = false;
        match a {
            AgentKind::ClaudeCode => {
                if claude_cli().is_some() {
                    removed_config = run_claude(&["mcp", "remove", SERVER, "-s", "user"]).is_ok_and(|(ok, _)| ok);
                }
            }
            AgentKind::Print => {}
            _ => removed_config = json_unregister(&config_path(a).unwrap())?,
        }
        let revoked = owner_call(&Request::AuthRevoke { name: app_name(a).to_owned() }).is_ok();
        results.push(json!({ "agent": a.slug(), "config_removed": removed_config, "token_revoked": revoked }));
        if !json_mode {
            ui::ok(format!(
                "{}: {}, {}",
                ui::bold(a.title()),
                if removed_config { "removed from the config" } else { "not in the config" },
                if revoked { "token revoked" } else { "no token" }
            ));
        }
    }
    if json_mode {
        println!("{}", serde_json::to_string_pretty(&json!({ "agents": results }))?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("discordctl-mcp-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    }

    #[test]
    fn registers_next_to_existing_servers_and_keeps_a_backup() {
        let p = temp("merge");
        std::fs::write(&p, r#"{"theme":"dark","mcpServers":{"other":{"command":"x"}}}"#).unwrap();
        json_register(&p, "dctl_abc").unwrap();
        let v = read_json(&p).unwrap();
        assert_eq!(v["theme"], "dark", "unrelated settings survive");
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"][SERVER]["env"]["DISCORDCTL_TOKEN"], "dctl_abc");
        assert!(p.with_extension("json.discordctl.bak").is_file(), "the original file is backed up once");
        assert!(json_has(&p));
        // Повторная запись заменяет токен, а не дублирует сервер.
        json_register(&p, "dctl_new").unwrap();
        assert_eq!(read_json(&p).unwrap()["mcpServers"][SERVER]["env"]["DISCORDCTL_TOKEN"], "dctl_new");
        assert!(json_unregister(&p).unwrap());
        assert!(!json_has(&p));
        assert_eq!(read_json(&p).unwrap()["mcpServers"]["other"]["command"], "x", "other servers stay");
        assert!(!json_unregister(&p).unwrap(), "second removal is a no-op");
    }

    #[test]
    fn creates_a_missing_config() {
        let p = temp("create");
        json_register(&p, "dctl_x").unwrap();
        assert!(json_has(&p));
    }

    #[test]
    fn never_overwrites_a_config_it_cannot_parse() {
        let p = temp("broken");
        std::fs::write(&p, "{ not json").unwrap();
        assert!(json_register(&p, "dctl_x").is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ not json", "the broken file is left untouched");
    }

    #[test]
    fn presets_are_valid_and_read_only_cannot_write() {
        let ro = preset_permissions(Preset::ReadOnly);
        assert!(ro.validate().is_ok() && ro.write.is_empty());
        assert!(
            ro.list.contains(&"guilds".to_owned()) && ro.list.contains(&"dms".to_owned()),
            "friendly list words are normalized"
        );
        let asst = preset_permissions(Preset::Assistant);
        assert!(asst.validate().is_ok() && !asst.write.is_empty());
        assert!(asst.needs_confirmation("write"), "assistant writes still wait for the owner");
    }

    #[test]
    fn quoting_for_cmd() {
        assert_eq!(quote("plain"), "plain");
        assert!(quote("a b").starts_with('"') && quote("a b").ends_with('"'));
        assert_eq!(quote("").len(), 2, "an empty argument is passed as two quote characters");
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;

    /// Claude Desktop из Microsoft Store хранит настройки в каталоге пакета: его нужно и находить, и предпочитать.
    #[test]
    fn claude_desktop_from_the_store_is_found_and_preferred() {
        let tmp = std::env::temp_dir().join(format!("dctl-claude-{}", std::process::id()));
        let (local, roaming) = (tmp.join("Local"), tmp.join("Roaming"));
        // Окружение процесса не трогаем: тесты идут параллельно, а пути читают и другие модули.
        assert!(!claude_desktop_installed_in(&local, &roaming));

        // Остаток после удаления (только %APPDATA%\Claude) установкой не считается.
        std::fs::create_dir_all(roaming.join("Claude")).unwrap();
        assert!(!claude_desktop_installed_in(&local, &roaming));
        std::fs::create_dir_all(local.join("AnthropicClaude")).unwrap();
        assert!(claude_desktop_installed_in(&local, &roaming));
        assert_eq!(claude_desktop_dirs_in(&local, &roaming), vec![roaming.join("Claude")]);

        let pkg = local.join("Packages").join("Claude_abc123").join("LocalCache").join("Roaming").join("Claude");
        std::fs::create_dir_all(&pkg).unwrap();
        assert_eq!(claude_desktop_dirs_in(&local, &roaming).first(), Some(&pkg));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Набор `Autonomous` даёт все права и отключает подтверждения; остальные наборы подтверждения оставляют.
    #[test]
    fn autonomous_preset_turns_every_approval_off_and_the_others_do_not() {
        let auto = preset_permissions(Preset::Autonomous);
        assert!(auto.validate().is_ok(), "the rights must be a valid combination");
        for scope in ["write", "voice", "stream"] {
            assert!(!auto.needs_confirmation(scope), "{scope} must not ask");
        }
        assert!(auto.allows_word("voice", "join") && auto.allows_word("stream", "screen"));
        for p in [Preset::ReadOnly, Preset::Assistant] {
            let perms = preset_permissions(p);
            assert!(perms.validate().is_ok());
            assert!(perms.needs_confirmation("write") && perms.needs_confirmation("voice"));
            assert!(perms.voice.is_empty() && perms.stream.is_empty());
        }
        assert!(preset_permissions(Preset::ReadOnly).write.is_empty());
        assert_eq!(preset_permissions(Preset::Assistant).write, vec!["*"]);
    }
}
