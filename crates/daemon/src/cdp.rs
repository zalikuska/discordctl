//! CDP-клиент поверх анонимных пайпов Chromium: сообщения JSON, разделённые `\0`.
//!
//! Читающий поток раскладывает ответы по ожидающим вызовам (по `id`), а события отдаёт
//! наружу через канал. Так обработчик событий может сам делать блокирующие вызовы.

use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

const CALL_TIMEOUT: Duration = Duration::from_secs(15);

type Reply = Result<Value, String>;

pub struct Cdp {
    /// Держим открытым: закрытие пайпа завершает Discord.
    writer: Mutex<File>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, mpsc::Sender<Reply>>>,
    closed: AtomicBool,
}

impl Cdp {
    /// Запускает читающий поток. Канал событий закрывается, когда Chromium закрыл пайп.
    pub fn start(to_browser: File, mut from_browser: File) -> (Arc<Cdp>, mpsc::Receiver<Value>) {
        let cdp = Arc::new(Cdp {
            writer: Mutex::new(to_browser),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        });
        let (events_tx, events_rx) = mpsc::channel();
        let reader = cdp.clone();
        thread::spawn(move || {
            let mut acc: Vec<u8> = Vec::new();
            let mut buf = [0u8; 64 * 1024];
            while let Ok(n) = from_browser.read(&mut buf) {
                if n == 0 {
                    break;
                }
                acc.extend_from_slice(&buf[..n]);
                for msg in drain_messages(&mut acc) {
                    reader.dispatch(msg, &events_tx);
                }
            }
            reader.closed.store(true, Ordering::SeqCst);
            // Отправители исчезают, ожидающие вызовы получают «pipe closed».
            reader.pending.lock().unwrap().clear();
        });
        (cdp, events_rx)
    }

    fn dispatch(&self, msg: Value, events: &mpsc::Sender<Value>) {
        if let Some(id) = msg.get("id").and_then(Value::as_u64) {
            let Some(tx) = self.pending.lock().unwrap().remove(&id) else { return };
            let reply = match msg.get("error") {
                Some(e) => Err(e.get("message").and_then(Value::as_str).unwrap_or("CDP error").to_owned()),
                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(reply);
        } else if msg.get("method").is_some() {
            let _ = events.send(msg);
        }
    }

    /// Вызов метода CDP. `session` — `sessionId` присоединённой цели (None — уровень браузера).
    pub fn call(&self, method: &str, params: Value, session: Option<&str>) -> Reply {
        self.call_timeout(method, params, session, CALL_TIMEOUT)
    }

    /// То же с своим сроком: чтение длинной истории идёт десятки секунд из-за пауз между запросами.
    pub fn call_timeout(&self, method: &str, params: Value, session: Option<&str>, timeout: Duration) -> Reply {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if self.closed.load(Ordering::SeqCst) {
            self.pending.lock().unwrap().remove(&id);
            return Err("pipe closed".into());
        }
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let mut bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
        bytes.push(0);
        if let Err(e) = self.writer.lock().unwrap().write_all(&bytes) {
            self.pending.lock().unwrap().remove(&id);
            return Err(format!("pipe write failed: {e}"));
        }
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("{method}: timeout"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("pipe closed".into()),
        }
    }
}

/// Вынимает из накопленных байтов все завершённые сообщения (до `\0`). Битый JSON пропускается.
fn drain_messages(acc: &mut Vec<u8>) -> Vec<Value> {
    let mut out = Vec::new();
    while let Some(pos) = acc.iter().position(|&b| b == 0) {
        let msg: Vec<u8> = acc.drain(..=pos).collect();
        if let Ok(v) = serde_json::from_slice::<Value>(&msg[..msg.len() - 1]) {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_messages_across_reads() {
        let mut acc = br#"{"id":1}"#.to_vec();
        assert!(drain_messages(&mut acc).is_empty());
        acc.extend_from_slice(b"\0{\"method\":\"A.b\"}\0{\"id\"");
        let msgs = drain_messages(&mut acc);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["id"], 1);
        assert_eq!(msgs[1]["method"], "A.b");
        assert_eq!(acc, b"{\"id\"");
    }

    #[test]
    fn skips_garbage() {
        let mut acc = b"not json\0{\"id\":2}\0".to_vec();
        let msgs = drain_messages(&mut acc);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["id"], 2);
    }
}
