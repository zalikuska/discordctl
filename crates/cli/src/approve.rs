//! Одобрение прав и подтверждение вызовов человеком.
//!
//! Ответы читаются с консоли (`CONIN$`), а не из stdin, и только если stdin и stdout — терминал:
//! `echo y | discordctl app add …` из скрипта или из оболочки агента без терминала не сработает.
//! Это защита от случайности и от простых оболочек, но не от агента с настоящим терминалом: см. DESIGN.md.

use crate::{
    api::{self, payload_call},
    ui,
};
use anyhow::{bail, Context, Result};
use console::style;
use discordctl_core::permissions::{valid_resource_pattern, Permissions};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::{IsTerminal, Write},
    time::Duration,
};

pub fn require_terminal() -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("this needs an interactive terminal: run the command yourself in a console window (approval cannot be piped)");
    }
    Ok(())
}

/// Вопрос человеку: одна клавиша, без Enter. Читаются нажатия клавиш консоли (Unicode), а не байты stdin:
/// так работают русская раскладка (д/н) и любая кодовая страница.
pub fn ask(question: &str, default_yes: bool) -> Result<bool> {
    use crossterm::{
        event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
        terminal,
    };
    require_terminal()?;
    print!("{} {} ", crate::i18n::tr(question), ui::dim(if default_yes { "[Y/n]" } else { "[y/N]" }));
    std::io::stdout().flush()?;
    terminal::enable_raw_mode().context("cannot open the console for input")?;
    let read_answer = || -> Result<Option<bool>> {
        // Нажатия, сделанные до появления вопроса (в очереди ввода), к нему не относятся: иначе случайное «y»
        // подтвердило бы чужой вызов.
        while event::poll(Duration::ZERO)? {
            event::read()?;
        }
        loop {
            let Event::Key(k) = event::read().context("cannot read from the console")? else { continue };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match k.code {
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(None),
                KeyCode::Enter => return Ok(Some(default_yes)),
                KeyCode::Esc => return Ok(Some(false)),
                KeyCode::Char(c) => match c.to_lowercase().next().unwrap_or(c) {
                    'y' | 'д' => return Ok(Some(true)),
                    'n' | 'н' => return Ok(Some(false)),
                    _ => {}
                },
                _ => {}
            }
        }
    };
    // Режим консоли возвращаем и при ошибке чтения.
    let answer = read_answer();
    let _ = terminal::disable_raw_mode();
    let answer = answer?;
    match answer {
        Some(yes) => {
            println!("{}", crate::i18n::tr(if yes { "yes" } else { "no" }));
            Ok(yes)
        }
        None => {
            println!();
            bail!("cancelled")
        }
    }
}

/// Приводит удобную запись цели к внутренней: `@имя` → `dm:имя`, `Сервер/*` → `guild:Сервер/channel:*`.
pub fn normalize_pattern(p: &str) -> String {
    if p == "*" || p.starts_with("dm:") || p.starts_with("guild:") || valid_resource_pattern(p) {
        return p.to_owned();
    }
    if p == "@*" {
        return "dm:*".into();
    }
    if let Some(name) = p.strip_prefix('@') {
        return format!("dm:{name}");
    }
    match p.split_once('/') {
        Some((g, "*")) => format!("guild:{g}/channel:*"),
        Some((g, c)) => format!("guild:{g}/channel:{c}"),
        None => p.to_owned(),
    }
}

/// Имена в шаблоне превращаем в id (права хранятся по id). Возвращает шаблон и подпись для человека.
fn canonicalize(pattern: &str, voice: bool) -> Result<(String, String)> {
    let pattern = normalize_pattern(pattern);
    let pattern = pattern.as_str();
    if pattern == "*" || pattern == "dm:*" || valid_resource_pattern(pattern) {
        return Ok((pattern.to_owned(), String::new()));
    }
    let resolve =
        |reference: &str| -> Result<Value> { payload_call("resolve", json!({ "channel": reference, "voice": voice })) };
    let channel_pattern = |v: &Value| -> (String, String) {
        let id = v["id"].as_str().unwrap_or("?");
        let pat = match v["guild_id"].as_str() {
            Some(g) => format!("guild:{g}/channel:{id}"),
            None => format!("dm:{id}"),
        };
        (pat, format!("{} {}", v["kind"].as_str().unwrap_or("?"), v["name"].as_str().unwrap_or("?")))
    };
    let guild_id = |name: &str| -> Result<(String, String)> {
        let list = payload_call("guilds", json!({}))?;
        let q = name.to_lowercase();
        let hits: Vec<&Value> = list
            .as_array()
            .into_iter()
            .flatten()
            .filter(|g| g["id"] == name || g["name"].as_str().is_some_and(|n| n.to_lowercase() == q))
            .collect();
        match hits.as_slice() {
            [g] => Ok((g["id"].as_str().unwrap_or("").to_owned(), g["name"].as_str().unwrap_or("").to_owned())),
            [] => bail!("server {name:?} not found"),
            _ => bail!("server name {name:?} is ambiguous, use its id"),
        }
    };
    if let Some(name) = pattern.strip_prefix("dm:") {
        return Ok(channel_pattern(&resolve(&format!("dm:{name}"))?));
    }
    if let Some(rest) = pattern.strip_prefix("guild:") {
        return match rest.split_once('/') {
            None => {
                let (id, name) = guild_id(rest)?;
                Ok((format!("guild:{id}"), format!("all channels of server {name}")))
            }
            Some((g, c)) => {
                let c = c.strip_prefix("channel:").unwrap_or(c);
                if c == "*" {
                    let (id, name) = guild_id(g)?;
                    Ok((format!("guild:{id}/channel:*"), format!("all channels of server {name}")))
                } else {
                    Ok(channel_pattern(&resolve(&format!("{g}/{c}"))?))
                }
            }
        };
    }
    bail!("{pattern:?} is not a target: use @name, Server/channel, Server/*, @* or an id")
}

/// Показывает запрошенные права построчно, даёт убрать лишнее и подтвердить целиком.
/// Возвращает окончательные права или `None`, если человек отказал.
pub fn review(app: &str, description: Option<&str>, requested: &Permissions) -> Result<Option<Permissions>> {
    require_terminal()?;
    ui::say(format!("\n{} asks for access to your Discord account.", style(format!("App \"{app}\"")).bold()));
    if let Some(d) = description {
        println!("{}", ui::dim(format!("It says: {d}")));
    }
    println!(
        "{}\n",
        ui::dim("Answer for each line. Rights are stored by id, so renaming a chat later changes nothing.")
    );
    let mut out = Permissions { confirm: requested.confirm.clone(), ..Default::default() };
    let mut asked = 0;

    let mut handle = |scope: &str,
                      items: &[String],
                      target: &mut Vec<String>,
                      is_resource: bool,
                      voice_channels: bool|
     -> Result<()> {
        for item in items {
            asked += 1;
            let (value, label) =
                if is_resource { canonicalize(item, voice_channels)? } else { (item.clone(), String::new()) };
            let shown = if label.is_empty() {
                ui::bold(&value)
            } else {
                format!("{}  {}", ui::bold(&value), ui::dim(format!("({label})")))
            };
            let tag = match scope {
                "write" | "stream" | "voice" | "vchan" => ui::yellow(format!("{scope:<6}")),
                _ => ui::cyan(format!("{scope:<6}")),
            };
            if ask(&format!("  {tag} {shown}  allow?"), true)? && !target.contains(&value) {
                target.push(value);
            }
        }
        Ok(())
    };
    handle("list", &requested.list, &mut out.list, false, false)?;
    handle("read", &requested.read, &mut out.read, true, false)?;
    handle("write", &requested.write, &mut out.write, true, false)?;
    handle("voice", &requested.voice, &mut out.voice, false, false)?;
    handle("vchan", &requested.voice_channels, &mut out.voice_channels, true, true)?;
    handle("stream", &requested.stream, &mut out.stream, false, false)?;

    if asked == 0 {
        println!("{}", ui::dim("(it asks for nothing: the app will only be able to ping)"));
    }
    out.validate().map_err(anyhow::Error::msg)?;
    let confirm = match &out.confirm {
        None => "write, voice and sharing calls".to_owned(),
        Some(c) if c.is_empty() => ui::red("nothing — every allowed call runs without asking!"),
        Some(c) => c.join(", "),
    };
    ui::say(format!(
        "\nBefore each call you will be asked for: {confirm}. Answer those with {}.",
        ui::bold("discordctl app approve")
    ));
    if !ask(&ui::bold("Grant exactly this?"), false)? {
        return Ok(None);
    }
    Ok(Some(out))
}

/// Просмотр набора прав одним вопросом: список строк и общее согласие. Для готовых наборов (preset), где строк много.
pub fn review_compact(app: &str, description: &str, requested: &Permissions) -> Result<Option<Permissions>> {
    require_terminal()?;
    ui::say(format!(
        "\n{} asks for access to your Discord account ({}).",
        style(format!("App \"{app}\"")).bold(),
        description
    ));
    let line = |label: &str, items: &[String], risky: bool| {
        if !items.is_empty() {
            let tag = if risky { ui::yellow(format!("{label:<6}")) } else { ui::cyan(format!("{label:<6}")) };
            println!("  {tag} {}", items.join(", "));
        }
    };
    line("list", &requested.list, false);
    line("read", &requested.read, false);
    line("write", &requested.write, true);
    line("voice", &requested.voice, true);
    line("stream", &requested.stream, true);
    let mut perms = requested.clone();
    perms.normalize();
    perms.validate().map_err(anyhow::Error::msg)?;
    if perms.write.is_empty() && perms.voice.is_empty() && perms.stream.is_empty() {
        println!("{}", ui::dim("  read-only: nothing can be sent, said or shared"));
    } else if perms.confirm.as_ref().is_some_and(|c| c.is_empty()) {
        println!("{}", ui::red("  NO approval: every call runs at once, the agent acts on its own"));
    } else {
        println!(
            "{}",
            ui::dim("  every write, voice and sharing call waits for your approval (discordctl app approve)")
        );
    }
    println!("{}", ui::dim("  `*` means every chat and server you can see now and in the future"));
    if !ask(&ui::bold("Grant exactly this?"), false)? {
        return Ok(None);
    }
    Ok(Some(perms))
}

/// Бесконечный цикл ответов на запросы приложений. Выход по Ctrl+C.
pub fn approvals_loop() -> Result<()> {
    require_terminal()?;
    ui::say(format!("{} Waiting for apps to ask for approval (Ctrl+C to stop)...", ui::cyan("◉")));
    let mut seen: HashSet<u64> = HashSet::new();
    loop {
        for item in api::pending()? {
            let id = item["id"].as_u64().unwrap_or(0);
            if !seen.insert(id) {
                continue;
            }
            println!(
                "\n{} {}\n{}",
                style("request").yellow().bold(),
                ui::dim(format!("#{id}")),
                item["summary"].as_str().unwrap_or("")
            );
            let allow = ask(&ui::bold("Allow this one call?"), false)?;
            let done = api::decide(id, allow)?;
            if done {
                if allow {
                    ui::ok("allowed")
                } else {
                    ui::fail("rejected")
                }
            } else {
                ui::warn("too late: it was decided or timed out");
            }
        }
        std::thread::sleep(Duration::from_millis(700));
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_pattern as n;

    #[test]
    fn friendly_targets_become_internal_patterns() {
        assert_eq!(n("*"), "*");
        assert_eq!(n("@*"), "dm:*");
        assert_eq!(n("@alice"), "dm:alice");
        assert_eq!(n("@Group name"), "dm:Group name");
        assert_eq!(n("My Server/*"), "guild:My Server/channel:*");
        assert_eq!(n("My Server/general"), "guild:My Server/channel:general");
        // Уже внутренняя запись не меняется.
        assert_eq!(n("dm:alice"), "dm:alice");
        assert_eq!(n("guild:123456789012345678/channel:*"), "guild:123456789012345678/channel:*");
        assert_eq!(n("123456789012345678"), "123456789012345678");
    }
}
