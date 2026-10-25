//! Приложения, токены, проверка вызовов, подтверждения человеком и журнал аудита.
//!
//! Границы модели (подробно в DESIGN.md): токен ограничивает того, кто им пользуется (MCP-сервер, скрипт).
//! Он не защищает от агента с обычной оболочкой того же пользователя: такой агент и так может позвать CLI
//! как владелец. Поэтому подключать агентов нужно через MCP с токеном, а не давая им CLI без токена.

use discordctl_core::{
    paths,
    permissions::{self, Permissions, Resource},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    sync::{Condvar, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};

/// Сколько вызов ждёт решения человека, потом отказ.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub name: String,
    /// SHA-256 токена; сам токен не хранится.
    token_hash: String,
    pub created: u64,
    pub permissions: Permissions,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    apps: Vec<App>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn store_path() -> std::path::PathBuf {
    paths::data_dir().join("apps.json")
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut guard = STORE.lock().unwrap();
    let store = guard.get_or_insert_with(|| {
        fs::read_to_string(store_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    });
    f(store)
}

fn save(store: &Store) -> Result<(), String> {
    let dir = paths::data_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Сначала во временный файл: оборванная запись не должна стереть права.
    let tmp = dir.join("apps.json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(store).unwrap()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, store_path()).map_err(|e| e.to_string())
}

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    let status = unsafe {
        BCryptGenRandom(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG)
    };
    if status != 0 {
        return Err(format!("system random generator failed (0x{status:x})"));
    }
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

// ---- журнал -------------------------------------------------------------------------------------------------

/// Одна строка JSON на событие. Текст сообщений в журнал не попадает, только длина и итог.
pub fn audit(app: &str, method: &str, resource: Option<&str>, decision: &str, detail: &str) {
    let line = json!({ "ts": now(), "app": app, "method": method, "resource": resource, "decision": decision, "detail": detail });
    let _ = fs::create_dir_all(paths::data_dir());
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(paths::data_dir().join("audit.log")) {
        let _ = writeln!(f, "{line}");
    }
}

pub fn audit_tail(limit: usize) -> Value {
    let text = fs::read_to_string(paths::data_dir().join("audit.log")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(limit.clamp(1, 1000));
    Value::Array(lines[from..].iter().filter_map(|l| serde_json::from_str(l).ok()).collect())
}

// ---- приложения ---------------------------------------------------------------------------------------------

/// Создаёт приложение или заменяет права существующего (токен при этом меняется). Возвращает токен, который больше не покажут.
pub fn grant(name: &str, perms: Permissions) -> Result<String, String> {
    permissions::valid_app_name(name)?;
    perms.validate()?;
    let token = format!("dctl_{}", random_hex(32)?);
    let app = App { name: name.to_owned(), token_hash: hash(&token), created: now(), permissions: perms.clone() };
    with_store(|s| {
        s.apps.retain(|a| a.name != name);
        s.apps.push(app);
        save(s)
    })?;
    audit("owner", "auth.grant", None, "granted", &format!("{name}: {}", summarize(&perms)));
    Ok(token)
}

pub fn revoke(name: &str) -> Result<bool, String> {
    let removed = with_store(|s| {
        let before = s.apps.len();
        s.apps.retain(|a| a.name != name);
        let removed = s.apps.len() != before;
        if removed {
            save(s)?;
        }
        Ok::<_, String>(removed)
    })?;
    if removed {
        audit("owner", "auth.revoke", None, "revoked", name);
        // Отзыв мгновенный, поэтому ждущие подтверждения этого приложения тоже снимаем.
        drop_pending_of(name);
    }
    Ok(removed)
}

pub fn list() -> Value {
    with_store(|s| {
        Value::Array(
            s.apps
                .iter()
                .map(|a| json!({ "name": a.name, "created": a.created, "permissions": a.permissions }))
                .collect(),
        )
    })
}

/// Приложение по токену. Хэши сравниваются целиком; токен 256 бит, подбор не имеет смысла.
pub fn identify(token: &str) -> Option<App> {
    let h = hash(token);
    with_store(|s| s.apps.iter().find(|a| a.token_hash == h).cloned())
}

fn summarize(p: &Permissions) -> String {
    format!("list={:?} read={:?} write={:?} voice={:?} stream={:?}", p.list, p.read, p.write, p.voice, p.stream)
}

// ---- подтверждения человеком -------------------------------------------------------------------------------

struct Pending {
    id: u64,
    app: String,
    summary: String,
    decision: Option<bool>,
}

static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
static DECIDED: Condvar = Condvar::new();
static NEXT_ID: Mutex<u64> = Mutex::new(1);

fn drop_pending_of(app: &str) {
    let mut p = PENDING.lock().unwrap();
    for item in p.iter_mut().filter(|i| i.app == app && i.decision.is_none()) {
        item.decision = Some(false);
    }
    DECIDED.notify_all();
}

pub fn pending() -> Value {
    let p = PENDING.lock().unwrap();
    Value::Array(
        p.iter()
            .filter(|i| i.decision.is_none())
            .map(|i| json!({ "id": i.id, "app": i.app, "summary": i.summary }))
            .collect(),
    )
}

pub fn decide(id: u64, allow: bool) -> bool {
    let mut p = PENDING.lock().unwrap();
    let Some(item) = p.iter_mut().find(|i| i.id == id && i.decision.is_none()) else { return false };
    item.decision = Some(allow);
    DECIDED.notify_all();
    true
}

/// Блокирует вызов до решения человека (`discordctl approvals`). Тишина дольше срока — отказ.
fn ask_human(app: &str, summary: String) -> Result<(), &'static str> {
    let id = {
        let mut n = NEXT_ID.lock().unwrap();
        *n += 1;
        *n - 1
    };
    let mut p = PENDING.lock().unwrap();
    p.push(Pending { id, app: app.to_owned(), summary, decision: None });
    let deadline = std::time::Instant::now() + APPROVAL_TIMEOUT;
    let verdict = loop {
        let decided = p.iter().find(|i| i.id == id).and_then(|i| i.decision);
        if let Some(d) = decided {
            break Some(d);
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break None;
        }
        p = DECIDED.wait_timeout(p, left).unwrap().0;
    };
    p.retain(|i| i.id != id);
    match verdict {
        Some(true) => Ok(()),
        Some(false) => Err("rejected by the user"),
        None => Err("no answer from the user in time"),
    }
}

// ---- авторизация вызова -------------------------------------------------------------------------------------

enum Need {
    Nothing,
    Word(&'static str, &'static str),
    /// Слово права плюс подтверждение человеком каждого вызова (голос).
    VoiceAction(&'static str),
    /// Запуск трансляции: слово `window` или `screen` по виду источника и подтверждение с названием окна.
    StreamStart,
    /// Вход в голосовой канал: слово `join`, канал из `voice_channels` и подтверждение.
    VoiceJoin,
    Channel(&'static str),
}

/// Какие методы payload вообще доступны приложениям и что для них нужно. Всё остальное (диагностика, `describe`,
/// `find_modules`, сырой доступ) приложениям закрыто.
fn need(method: &str) -> Option<Need> {
    Some(match method {
        "ping" => Need::Nothing,
        "me" => Need::Word("list", "me"),
        "guilds" => Need::Word("list", "guilds"),
        "channels" => Need::Word("list", "channels"),
        "dms" => Need::Word("list", "dms"),
        "friends" => Need::Word("list", "friends"),
        "stream_sources" => Need::Word("stream", "list"),
        "stream_status" => Need::Word("stream", "status"),
        "stream_stop" => Need::Word("stream", "stop"),
        "stream_start" => Need::StreamStart,
        "voice_status" => Need::Word("voice", "status"),
        "voice_join" => Need::VoiceJoin,
        "voice_leave" => Need::VoiceAction("leave"),
        "voice_mute" => Need::VoiceAction("mute"),
        "voice_deafen" => Need::VoiceAction("deafen"),
        "messages" => Need::Channel("read"),
        "send" | "edit" | "delete" | "react" => Need::Channel("write"),
        _ => return None,
    })
}

/// Разбирает ссылку на канал в канонический ресурс. Ошибки скрываем: подробное сообщение («неоднозначно: …»)
/// выдало бы приложению имена чатов, которые ему не разрешены.
fn resolve(reference: &str, voice: bool) -> Result<(Resource, String), String> {
    let v = crate::hook::call("resolve", json!({ "channel": reference, "voice": voice }))
        .map_err(|_| "channel not found or not available".to_owned())?;
    let id = v["id"].as_str().ok_or("channel not found or not available")?.to_owned();
    let guild_id = v["guild_id"].as_str().map(str::to_owned);
    let name = format!("{} {}", v["kind"].as_str().unwrap_or("channel"), v["name"].as_str().unwrap_or("?"));
    Ok((Resource { guild_id, channel_id: id }, name))
}

fn human_summary(app: &str, method: &str, place: &str, params: &Value) -> String {
    let text = params["content"].as_str().unwrap_or("");
    match method {
        "send" => format!("{app} wants to SEND to {place}:\n{text}"),
        "edit" => format!("{app} wants to EDIT message {} in {place} to:\n{text}", params["message"].as_str().unwrap_or("?")),
        "delete" => format!("{app} wants to DELETE message {} in {place}", params["message"].as_str().unwrap_or("?")),
        "react" => format!(
            "{app} wants to {} reaction {} on message {} in {place}",
            if params["remove"] == true { "REMOVE" } else { "ADD" },
            params["emoji"].as_str().unwrap_or("?"),
            params["message"].as_str().unwrap_or("?")
        ),
        "voice_join" => format!(
            "{app} wants to JOIN the voice channel {place}{}",
            if params["ring"] == true { " (this may RING people)" } else { "" }
        ),
        "stream_start" => format!(
            "{app} wants to START STREAMING {place} to the voice channel you are in. Everyone who can see that channel can watch what is on it{}",
            if params["audio"] == true { ", WITH SOUND" } else { "" }
        ),
        "voice_leave" => format!("{app} wants to LEAVE the current voice channel"),
        "voice_mute" => format!("{app} wants to {} your microphone", if params["mute"] == false { "UNMUTE" } else { "MUTE" }),
        "voice_deafen" => format!("{app} wants to {} your sound", if params["deafen"] == false { "UNDEAFEN" } else { "DEAFEN" }),
        _ => format!("{app} wants to run {method} in {place}"),
    }
}

/// Проверяет вызов приложения, при необходимости ждёт человека и исполняет. Каждое решение попадает в журнал.
pub fn app_call(app: &App, method: &str, params: Value) -> Result<Value, String> {
    let deny = |why: &str, res: Option<&str>| -> Result<Value, String> {
        audit(&app.name, method, res, "denied", why);
        Err(why.to_owned())
    };
    let Some(need) = need(method) else { return deny("method is not available to apps", None) };
    let mut params = params;
    let mut resource_label: Option<String> = None;
    let mut place = String::new();
    let mut scope: Option<&'static str> = None;
    match need {
        Need::Nothing => {}
        Need::Word(sc, word) => {
            if !app.permissions.allows_word(sc, word) {
                return deny(&format!("no permission: {sc} {word}"), None);
            }
        }
        Need::VoiceAction(word) => {
            if !app.permissions.allows_word("voice", word) {
                return deny(&format!("no permission: voice {word}"), None);
            }
            scope = Some("voice");
        }
        Need::StreamStart => {
            let source = params["source"].as_str().unwrap_or("").to_owned();
            let kind = if source.starts_with("screen:") { "screen" } else { "window" };
            if !app.permissions.allows_word("stream", kind) {
                return deny(&format!("no permission: stream {kind}"), None);
            }
            // Название окна для человека берём из самого клиента (сухой запуск ничего не делает).
            let plan = match crate::hook::call("stream_start", json!({ "source": source, "dry_run": true })) {
                Ok(p) => p,
                Err(e) => return deny(&e, None),
            };
            if plan["voice"].is_null() {
                return deny("not in a voice channel: join one first", None);
            }
            place = format!(
                "the {} \"{}\" (channel: {})",
                kind,
                plan["source"]["name"].as_str().unwrap_or("?"),
                plan["voice"]["name"].as_str().unwrap_or("?")
            );
            resource_label = Some(source);
            scope = Some("stream");
        }
        Need::VoiceJoin => {
            if !app.permissions.allows_word("voice", "join") {
                return deny("no permission: voice join", None);
            }
            let reference = params["channel"].as_str().unwrap_or("");
            let (res, name) = match resolve(reference, true) {
                Ok(r) => r,
                Err(e) => return deny(&e, None),
            };
            let label = res.pattern();
            if !app.permissions.allows_resource("voice_channels", &res) {
                return deny(&format!("no permission: voice channel {label}"), Some(&label));
            }
            params["channel"] = json!(res.channel_id);
            resource_label = Some(label);
            place = name;
            scope = Some("voice");
        }
        Need::Channel(sc) => {
            let reference = params["channel"].as_str().unwrap_or("");
            let (res, name) = match resolve(reference, false) {
                Ok(r) => r,
                Err(e) => return deny(&e, None),
            };
            let label = res.pattern();
            if !app.permissions.allows_resource(sc, &res) {
                return deny(&format!("no permission: {sc} {label}"), Some(&label));
            }
            // Дальше работаем с каноническим id: что проверили, то и исполняем.
            params["channel"] = json!(res.channel_id);
            resource_label = Some(label);
            place = name;
            scope = Some(sc);
        }
    }
    if let Some(sc) = scope {
        if app.permissions.needs_confirmation(sc) && params["dry_run"] != true {
            let summary = human_summary(&app.name, method, &place, &params);
            if let Err(why) = ask_human(&app.name, summary) {
                return deny(why, resource_label.as_deref());
            }
        }
    }
    // Права могли отозвать, пока ждали человека.
    if identify_name(&app.name).is_none() {
        return deny("app was revoked", resource_label.as_deref());
    }
    let detail =
        params["content"].as_str().map(|c| format!("content: {} chars", c.chars().count())).unwrap_or_default();
    match crate::hook::call(method, params) {
        Ok(v) => {
            audit(&app.name, method, resource_label.as_deref(), "allowed", &detail);
            Ok(v)
        }
        Err(e) => {
            audit(&app.name, method, resource_label.as_deref(), "error", &e);
            Err(e)
        }
    }
}

fn identify_name(name: &str) -> Option<()> {
    with_store(|s| s.apps.iter().find(|a| a.name == name).map(|_| ()))
}
