//! Разговор с демоном и общие помощники команд.

use anyhow::{bail, Result};
use discordctl_core::{
    client,
    ipc::{self, ClientError},
    paths,
    protocol::{Request, Response},
    shim::{self, Target},
};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};

pub const T: Duration = Duration::from_secs(10);
/// Чтение истории идёт с паузами между страницами, запись ждёт человека до двух минут.
pub const LONG: Duration = Duration::from_secs(160);

/// Запрос к уже работающему демону. `None`, если демон не запущен.
pub fn ask(req: &Request) -> Result<Option<Response>> {
    match ipc::request(&paths::pipe_name(), req, T) {
        Ok(r) => Ok(Some(r)),
        Err(ClientError::NotRunning) => Ok(None),
        Err(e) => bail!("{e}"),
    }
}

/// Запрос с запуском демона при необходимости; ошибка демона становится ошибкой команды.
pub fn owner_call(req: &Request) -> Result<Response> {
    let r = client::call(req, T).map_err(anyhow::Error::msg)?;
    if !r.ok {
        bail!("{}", r.msg.clone().unwrap_or_default());
    }
    Ok(r)
}

/// Вызов метода payload внутри Discord.
pub fn payload_call(method: &str, params: Value) -> Result<Value> {
    let r = client::call(&Request::Call { method: method.to_owned(), params }, LONG).map_err(anyhow::Error::msg)?;
    if !r.ok {
        bail!("{}", r.msg.unwrap_or_default());
    }
    Ok(r.data.unwrap_or(Value::Null))
}

pub fn pending() -> Result<Vec<Value>> {
    Ok(owner_call(&Request::Approvals)?.data.and_then(|d| d.as_array().cloned()).unwrap_or_default())
}

pub fn decide(id: u64, allow: bool) -> Result<bool> {
    Ok(client::call(&Request::Decide { id, allow }, T).map_err(anyhow::Error::msg)?.ok)
}

/// Остановка демона. Без `force` отказывает, пока управляемый Discord запущен: закрытие пайпа закрыло бы и Discord.
pub fn stop_daemon(force: bool) -> Result<()> {
    match ask(&Request::Shutdown { force })? {
        None => Ok(()),
        Some(r) if r.ok => {
            std::thread::sleep(Duration::from_millis(500));
            Ok(())
        }
        Some(r) => bail!("{}", r.msg.unwrap_or_default()),
    }
}

/// Файл рядом с `discordctl.exe` (собранный или распакованный из релиза).
pub fn sibling(name: &str) -> Result<PathBuf> {
    let p = std::env::current_exe()?.with_file_name(name);
    if !p.is_file() {
        bail!(
            "{} not found next to discordctl.exe (unpack the whole release, or build the whole workspace)",
            p.display()
        );
    }
    Ok(p)
}

pub fn shim_status_value(root: &std::path::Path) -> Value {
    json!({ "root": root, "update": shim::state(root, Target::Update), "stub": shim::state(root, Target::Stub) })
}

/// Печать для `--json` или человека.
pub fn out(json_mode: bool, value: &Value, human: impl FnOnce()) {
    if json_mode {
        println!("{}", serde_json::to_string_pretty(value).unwrap());
    } else {
        human();
    }
}

/// Текст сообщения из аргументов или stdin. Пустой текст не отправляем.
pub fn message_text(words: &[String], stdin: bool) -> Result<String> {
    use anyhow::Context;
    let text = if stdin {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).context("reading stdin")?;
        buf.trim_end_matches(['\r', '\n']).to_owned()
    } else {
        words.join(" ")
    };
    if text.trim().is_empty() {
        bail!("the message is empty: write it after the target, or pipe it in with --stdin");
    }
    Ok(text)
}
