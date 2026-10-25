//! Надзор за хуком: находит главное окно Discord, ставит payload до загрузки страницы и следит за его состоянием.
//!
//! Сплэш и основное окно — разные цели CDP. Хук нужен только основному (страница на discord.com).

use crate::cdp::Cdp;
use discordctl_core::paths;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{mpsc::Receiver, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// Бандл из `payload/` (`npm run build`). Лежит в репозитории готовым: для сборки демона Node не нужен.
const PAYLOAD: &str = include_str!("../../../payload/dist/payload.js");
const PROBE: &str = "window.__discordctl ? JSON.stringify(window.__discordctl.status()) : null";
/// Сколько ждём готовности payload после перезагрузки страницы, прежде чем считать хук сломанным.
const READY_DEADLINE: Duration = Duration::from_secs(60);
/// Потолок одного вызова payload: история на 2000 сообщений идёт ~20 запросов с паузами.
pub const CALL_DEADLINE: Duration = Duration::from_secs(150);

#[derive(Clone)]
struct HookState {
    /// idle | waiting_page | installing | waiting_payload | active | broken | failed
    phase: &'static str,
    target_id: Option<String>,
    session_id: Option<String>,
    payload: Option<Value>,
    error: Option<String>,
    /// Канал к Discord, нужен для `call`.
    cdp: Option<Arc<Cdp>>,
    /// Идентификатор скрипта нового документа, нужен для замены payload.
    script_id: Option<String>,
}

const IDLE: HookState = HookState {
    phase: "idle",
    target_id: None,
    session_id: None,
    payload: None,
    error: None,
    cdp: None,
    script_id: None,
};

static HOOK: Mutex<HookState> = Mutex::new(IDLE);

fn log(msg: &str) {
    paths::log("daemon", &format!("hook: {msg}"));
}

fn update(f: impl FnOnce(&mut HookState)) {
    f(&mut HOOK.lock().unwrap());
}

pub fn snapshot() -> Value {
    let s = HOOK.lock().unwrap().clone();
    json!({
        "phase": s.phase,
        "active": s.phase == "active",
        "target_id": s.target_id,
        "payload": s.payload,
        "error": s.error,
    })
}

pub fn reset() {
    *HOOK.lock().unwrap() = IDLE;
}

/// Вызывает метод payload (`window.__discordctl.call`) в основном окне и разбирает ответ `{ok, result|error}`.
pub fn call(method: &str, params: Value) -> Result<Value, String> {
    let (cdp, session) = {
        let s = HOOK.lock().unwrap();
        match (s.phase, s.cdp.clone(), s.session_id.clone()) {
            ("active", Some(cdp), Some(session)) => (cdp, session),
            (phase, ..) => return Err(format!("hook is not active (phase: {phase})")),
        }
    };
    // serde_json выдаёт корректные JS-литералы, подстановки вручную нет.
    let expression = format!("window.__discordctl.call({}, {})", json!(method), params);
    let r = cdp.call_timeout(
        "Runtime.evaluate",
        json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }),
        Some(&session),
        CALL_DEADLINE,
    )?;
    if let Some(ex) = r.get("exceptionDetails") {
        return Err(format!("payload threw: {}", ex["exception"]["description"].as_str().unwrap_or("unknown")));
    }
    let raw = r["result"]["value"].as_str().ok_or("payload returned no value")?;
    let reply: Value = serde_json::from_str(raw).map_err(|e| format!("bad payload reply: {e}"))?;
    if reply["ok"] == true {
        Ok(reply["result"].clone())
    } else {
        Err(reply["error"].as_str().unwrap_or("payload error").to_owned())
    }
}

/// Страница основного окна: discord.com (и ptb/canary), но не всплывающие окна и оверлей.
fn is_main_page(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    (host == "discord.com" || host.ends_with(".discord.com"))
        && !path.starts_with("popout")
        && !path.starts_with("overlay")
}

/// Обрабатывает события цели до закрытия пайпа. Блокирует поток.
pub fn run(cdp: Arc<Cdp>, events: Receiver<Value>) {
    update(|s| *s = HookState { phase: "waiting_page", ..IDLE });
    if let Err(e) = cdp.call("Target.setDiscoverTargets", json!({ "discover": true }), None) {
        update(|s| {
            s.phase = "failed";
            s.error = Some(format!("Target.setDiscoverTargets: {e}"));
        });
        log(&format!("setDiscoverTargets failed: {e}"));
        return;
    }
    // Цели, к которым уже пытались присоединиться: повторно не трогаем, иначе перезагрузка зациклится.
    let mut seen: HashSet<String> = HashSet::new();
    for ev in events {
        let params = &ev["params"];
        match ev["method"].as_str().unwrap_or("") {
            "Target.targetCreated" | "Target.targetInfoChanged" => {
                let info = &params["targetInfo"];
                let (Some(id), Some(url)) = (info["targetId"].as_str(), info["url"].as_str()) else { continue };
                if info["type"] != "page" || !is_main_page(url) {
                    continue;
                }
                if HOOK.lock().unwrap().session_id.is_some() || !seen.insert(id.to_owned()) {
                    continue; // главное окно уже под хуком
                }
                log(&format!("main page {id} ({url})"));
                match install(&cdp, id) {
                    Ok(session) => {
                        let (cdp, target) = (cdp.clone(), id.to_owned());
                        thread::spawn(move || poll(cdp, session, target));
                    }
                    Err(e) => {
                        log(&format!("install failed: {e}"));
                        update(|s| {
                            s.phase = "failed";
                            s.error = Some(e);
                        });
                    }
                }
            }
            "Target.targetDestroyed" | "Target.targetCrashed" => {
                let Some(id) = params["targetId"].as_str() else { continue };
                seen.remove(id);
                if HOOK.lock().unwrap().target_id.as_deref() == Some(id) {
                    log(&format!("main page {id} gone"));
                    update(|s| *s = HookState { phase: "waiting_page", ..IDLE });
                }
            }
            "Target.detachedFromTarget" => {
                let Some(session) = params["sessionId"].as_str() else { continue };
                if HOOK.lock().unwrap().session_id.as_deref() == Some(session) {
                    log("session detached");
                    update(|s| *s = HookState { phase: "waiting_page", ..IDLE });
                    seen.clear();
                }
            }
            _ => {}
        }
    }
    log("event stream closed");
    reset();
}

/// Присоединяется, ставит скрипт на каждый новый документ и перезагружает страницу.
fn install(cdp: &Arc<Cdp>, target_id: &str) -> Result<String, String> {
    update(|s| {
        s.phase = "installing";
        s.target_id = Some(target_id.to_owned());
        s.error = None;
        s.cdp = Some(cdp.clone());
    });
    let r = cdp.call("Target.attachToTarget", json!({ "targetId": target_id, "flatten": true }), None)?;
    let session = r["sessionId"].as_str().ok_or("attachToTarget: no sessionId")?.to_owned();
    update(|s| s.session_id = Some(session.clone()));
    // Без Page.enable скрипты нового документа не выполняются.
    cdp.call("Page.enable", json!({}), Some(&session))?;
    let script = cdp.call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": PAYLOAD }), Some(&session))?;
    update(|s| s.script_id = script["identifier"].as_str().map(str::to_owned));
    cdp.call("Page.reload", json!({}), Some(&session))?;
    update(|s| s.phase = "waiting_payload");
    Ok(session)
}

/// Заменяет payload и перезагружает страницу, не трогая сам Discord. Для разработки: `payload_path` — свежий бандл с диска.
/// Любой процесс пользователя может внедрить код в Discord, поэтому на шаге 7 запрос закроется правами.
pub fn reload(payload_path: Option<&str>) -> Result<(), String> {
    let source = match payload_path {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("cannot read {p}: {e}"))?,
        None => PAYLOAD.to_owned(),
    };
    let (cdp, session, old) = {
        let s = HOOK.lock().unwrap();
        match (s.cdp.clone(), s.session_id.clone()) {
            (Some(cdp), Some(session)) => (cdp, session, s.script_id.clone()),
            _ => return Err(format!("no hooked page (phase: {})", s.phase)),
        }
    };
    if let Some(id) = old {
        let _ = cdp.call("Page.removeScriptToEvaluateOnNewDocument", json!({ "identifier": id }), Some(&session));
    }
    let script = cdp.call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": source }), Some(&session))?;
    update(|s| {
        s.script_id = script["identifier"].as_str().map(str::to_owned);
        s.phase = "waiting_payload";
        s.payload = None;
    });
    cdp.call("Page.reload", json!({}), Some(&session))?;
    log("payload reloaded");
    Ok(())
}

/// Опрашивает payload. Живёт, пока сессия остаётся основной: если страница сама перезагрузится, статус обновится.
fn poll(cdp: Arc<Cdp>, session: String, target: String) {
    let mut since = Instant::now();
    let mut was_active = false;
    loop {
        thread::sleep(Duration::from_secs(if was_active { 3 } else { 1 }));
        if HOOK.lock().unwrap().session_id.as_deref() != Some(session.as_str()) {
            return;
        }
        let probe = cdp.call("Runtime.evaluate", json!({ "expression": PROBE, "returnByValue": true }), Some(&session));
        let payload = match &probe {
            Ok(r) => r["result"]["value"].as_str().and_then(|s| serde_json::from_str::<Value>(s).ok()),
            Err(e) if e == "pipe closed" => return,
            Err(_) => None, // страница перезагружается: контекста пока нет
        };
        let ready = payload.as_ref().is_some_and(|p| p["ready"] == true);
        if was_active && !ready {
            since = Instant::now(); // страница перезагрузилась, ждём заново
        }
        was_active = ready;
        let expired = since.elapsed() > READY_DEADLINE;
        update(|s| {
            if s.session_id.as_deref() != Some(session.as_str()) {
                return;
            }
            let (phase, error) = match (&payload, ready) {
                (_, true) => ("active", None),
                (Some(p), false) if expired => {
                    ("broken", Some(format!("payload loaded but required stores are missing: {}", p["missing"])))
                }
                (None, false) if expired => ("failed", Some("payload did not respond (hook did not run?)".to_owned())),
                _ => ("waiting_payload", None),
            };
            if s.phase != phase {
                log(&format!("{} -> {phase} (target {target})", s.phase));
            }
            s.phase = phase;
            s.error = error;
            if payload.is_some() {
                s.payload = payload.clone();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::is_main_page;

    #[test]
    fn matches_main_window_only() {
        assert!(is_main_page("https://discord.com/app"));
        assert!(is_main_page("https://discord.com/channels/@me"));
        assert!(is_main_page("https://canary.discord.com/channels/@me"));
        assert!(!is_main_page("https://discord.com/popout?x=1"));
        assert!(!is_main_page("https://discord.com/overlay"));
        assert!(!is_main_page("https://evildiscord.com/app"));
        assert!(!is_main_page("about:blank"));
        assert!(!is_main_page("file:///C:/splash.html"));
    }
}
