//! События Discord для приложений: новые сообщения, голосовые каналы, набор текста.
//!
//! Откуда берутся: payload внутри клиента подписан на внутренние события Discord и копит их в очереди; демон раз в
//! четверть секунды забирает очередь (`watch_drain`), пока кто-то слушает, и раскладывает по «почтовым ящикам».
//! У каждого слушателя свой ящик: ключ `app:<имя>` у приложения (все его MCP-процессы делят один) и `owner:<имя>`
//! у владельца. Ящик держит до `MAILBOX_MAX` событий (старые вытесняются, число потерянных сообщается) и живёт
//! `MAILBOX_IDLE` после последнего обращения: пока агент не опрашивает, события копятся, а забытый ящик исчезает.
//!
//! Что можно получить, решает демон, а не payload: сообщения и набор текста только из каналов, которые приложение
//! вправе читать (`read`), голос только тем, у кого в `list` есть слово `voices`.

use crate::hook;
use discordctl_core::permissions::{Permissions, Resource};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

pub const MESSAGE_KINDS: [&str; 3] = ["message", "message_edit", "message_delete"];
pub const VOICE_KINDS: [&str; 4] = ["voice_join", "voice_leave", "voice_move", "voice_state"];
/// Всё, что умеем отдавать. Присутствие шумное и касается других людей, поэтому в «всё» не входит: его просят явно.
pub const ALL_KINDS: [&str; 9] = [
    "message",
    "message_edit",
    "message_delete",
    "voice_join",
    "voice_leave",
    "voice_move",
    "voice_state",
    "typing",
    "presence",
];
pub const DEFAULT_KINDS: [&str; 8] =
    ["message", "message_edit", "message_delete", "voice_join", "voice_leave", "voice_move", "voice_state", "typing"];

const MAILBOX_MAX: usize = 500;
const MAILBOX_IDLE: Duration = Duration::from_secs(120);
const POLL_EVERY: Duration = Duration::from_millis(250);
const RESET_EVERY: Duration = Duration::from_secs(5);
/// Как часто просыпается ожидающий вызов, чтобы заметить, что клиент отключился.
const GONE_CHECK: Duration = Duration::from_millis(400);
pub const MAX_WAIT: u64 = 50;
pub const MAX_TAKE: usize = 100;

/// Группы видов: `messages`, `voice`, `all` или конкретный вид. Пусто и `all` значат «всё, кроме присутствия».
pub fn expand(requested: &[String]) -> Result<Vec<&'static str>, String> {
    if requested.is_empty() {
        return Ok(DEFAULT_KINDS.to_vec());
    }
    let mut out: Vec<&'static str> = vec![];
    for r in requested {
        let group: &[&'static str] = match r.as_str() {
            "all" => &DEFAULT_KINDS,
            "messages" => &MESSAGE_KINDS,
            "voice" => &VOICE_KINDS,
            other => match ALL_KINDS.iter().find(|k| **k == other) {
                Some(k) => std::slice::from_ref(k),
                None => {
                    return Err(format!(
                        "unknown event kind {other:?}: use {}, or messages, voice, all (all leaves presence out)",
                        ALL_KINDS.join(", ")
                    ))
                }
            },
        };
        for k in group {
            if !out.contains(k) {
                out.push(k);
            }
        }
    }
    Ok(out)
}

/// Кто слушает: владелец видит всё, приложение только то, на что у него права.
#[derive(Clone)]
pub enum Scope {
    Owner,
    App(Permissions),
}

impl Scope {
    /// Какие из запрошенных видов этому слушателю вообще можно.
    pub fn filter_kinds(&self, kinds: Vec<&'static str>) -> Vec<&'static str> {
        match self {
            Scope::Owner => kinds,
            Scope::App(p) => kinds
                .into_iter()
                .filter(|k| {
                    if VOICE_KINDS.contains(k) {
                        p.allows_word("list", "voices")
                    } else if *k == "presence" {
                        p.allows_word("list", "presence")
                    } else {
                        !p.read.is_empty()
                    }
                })
                .collect(),
        }
    }

    fn allows(&self, ev: &Value) -> bool {
        let Scope::App(p) = self else { return true };
        let kind = ev["kind"].as_str().unwrap_or("");
        if VOICE_KINDS.contains(&kind) {
            return p.allows_word("list", "voices");
        }
        if kind == "presence" {
            return p.allows_word("list", "presence");
        }
        let Some(channel_id) = ev["channel_id"].as_str() else { return false };
        // Сервер берём из события, а если там нет (у набора текста его не бывает), из описания канала. Без сервера
        // событие считается личным только при явном признаке: иначе канал сервера с пустым `guild_id` подошёл бы под
        // право `dm:*`. Нет уверенности, значит отказ.
        let guild_id = ev["guild_id"].as_str().or(ev["channel"]["guild_id"].as_str()).map(str::to_owned);
        if guild_id.is_none() && !matches!(ev["channel"]["kind"].as_str(), Some("dm" | "group")) {
            return false;
        }
        p.allows_resource("read", &Resource { guild_id, channel_id: channel_id.to_owned() })
    }
}

/// Условия подписки на присутствие. Пустое поле значит «любое»; заданные должны совпасть все.
/// `status`: человек вошёл в этот статус. `activity`: он начал или бросил занятие с таким названием (часть названия).
#[derive(Clone, Default, PartialEq, Debug)]
pub struct PresenceFilter {
    pub status: Vec<String>,
    pub activity: Vec<String>,
    pub users: Vec<String>,
}

const FILTER_ITEMS: usize = 20;
const FILTER_LEN: usize = 100;

impl PresenceFilter {
    pub fn parse(v: &Value) -> Result<PresenceFilter, String> {
        if v.is_null() {
            return Ok(PresenceFilter::default());
        }
        let obj = v
            .as_object()
            .ok_or("filter must be an object like {\"status\": [...], \"activity\": [...], \"users\": [...]}")?;
        for key in obj.keys() {
            if !["status", "activity", "users"].contains(&key.as_str()) {
                return Err(format!("filter.{key}: unknown field (use status, activity, users)"));
            }
        }
        let list = |key: &str, lower: bool| -> Result<Vec<String>, String> {
            let Some(raw) = obj.get(key).filter(|x| !x.is_null()) else { return Ok(vec![]) };
            // Одно слово вместо списка тоже годится.
            let items: Vec<&Value> = match raw.as_array() {
                Some(a) => a.iter().collect(),
                None => vec![raw],
            };
            if items.len() > FILTER_ITEMS {
                return Err(format!("filter.{key}: at most {FILTER_ITEMS} entries"));
            }
            items
                .into_iter()
                .map(|x| {
                    let s = x.as_str().ok_or(format!("filter.{key}: entries must be text"))?.trim();
                    if s.is_empty() || s.len() > FILTER_LEN {
                        return Err(format!("filter.{key}: an entry is empty or longer than {FILTER_LEN} characters"));
                    }
                    Ok(if lower { s.to_lowercase() } else { s.to_owned() })
                })
                .collect()
        };
        let status = list("status", true)?;
        if let Some(bad) = status.iter().find(|s| !["online", "idle", "dnd", "offline"].contains(&s.as_str())) {
            return Err(format!("filter.status: {bad:?} is not a status (online, idle, dnd, offline)"));
        }
        Ok(PresenceFilter { status, activity: list("activity", true)?, users: list("users", false)? })
    }

    pub fn is_empty(&self) -> bool {
        self.status.is_empty() && self.activity.is_empty() && self.users.is_empty()
    }

    pub fn to_json(&self) -> Value {
        json!({ "status": self.status, "activity": self.activity, "users": self.users })
    }

    /// Подходит ли событие присутствия (то же правило, что в payload: `presenceMatches`).
    pub fn matches(&self, ev: &Value) -> bool {
        if !self.users.is_empty() && !ev["user"]["id"].as_str().is_some_and(|id| self.users.iter().any(|u| u == id)) {
            return false;
        }
        if !self.status.is_empty() {
            let now = ev["status"].as_str().unwrap_or("");
            if !self.status.iter().any(|s| s == now) || ev["prev_status"].as_str() == Some(now) {
                return false;
            }
        }
        if !self.activity.is_empty() {
            let hit = ev["changes"].as_array().into_iter().flatten().any(|c| {
                let name = c["name"].as_str().unwrap_or("").to_lowercase();
                self.activity.iter().any(|w| name.contains(w.as_str()))
            });
            if !hit {
                return false;
            }
        }
        true
    }
}

struct Mailbox {
    scope: Scope,
    filter: PresenceFilter,
    kinds: HashSet<&'static str>,
    queue: VecDeque<Value>,
    dropped: u64,
    last_used: Instant,
}

static HUB: Mutex<Option<HashMap<String, Mailbox>>> = Mutex::new(None);
/// Почему события не идут, если насос не смог настроить подписку (например, Discord поменял внутренности).
static PROBLEM: Mutex<Option<String>> = Mutex::new(None);
/// Будит и ожидающие вызовы (пришли события), и насос (появился слушатель).
static CHANGED: Condvar = Condvar::new();

fn log(msg: &str) {
    discordctl_core::paths::log("daemon", &format!("events: {msg}"));
}

fn with_hub<T>(f: impl FnOnce(&mut HashMap<String, Mailbox>) -> T) -> T {
    let mut g = HUB.lock().unwrap();
    f(g.get_or_insert_with(HashMap::new))
}

/// Раскладывает события по ящикам тех, кому они положены.
pub fn publish(events: Vec<Value>) {
    if events.is_empty() {
        return;
    }
    with_hub(|boxes| {
        for ev in events {
            let Some(kind) = ev["kind"].as_str().map(str::to_owned) else { continue };
            for b in boxes.values_mut() {
                if b.kinds.contains(kind.as_str())
                    && b.scope.allows(&ev)
                    && (kind != "presence" || b.filter.matches(&ev))
                {
                    if b.queue.len() >= MAILBOX_MAX {
                        b.queue.pop_front();
                        b.dropped += 1;
                    }
                    b.queue.push_back(ev.clone());
                }
            }
        }
    });
    CHANGED.notify_all();
}

/// Приложение отозвано: его ящик и ждущие вызовы больше не нужны.
pub fn drop_mailbox(key: &str) {
    with_hub(|boxes| boxes.remove(key));
    CHANGED.notify_all();
}

/// Ждёт события для ящика `key` до `wait` и отдаёт не больше `max`. Ящик создаётся при первом обращении и дальше копит
/// события между вызовами. Виды и права берутся из каждого вызова: смена прав действует сразу.
pub fn wait(
    key: &str,
    scope: Scope,
    kinds: Vec<&'static str>,
    filter: PresenceFilter,
    wait: Duration,
    max: usize,
    gone: &dyn Fn() -> bool,
) -> Result<(Value, bool), String> {
    let deadline = Instant::now() + wait;
    let mut created = false;
    let mut guard = HUB.lock().unwrap();
    {
        let boxes = guard.get_or_insert_with(HashMap::new);
        let b = boxes.entry(key.to_owned()).or_insert_with(|| {
            created = true;
            Mailbox {
                scope: scope.clone(),
                filter: PresenceFilter::default(),
                kinds: HashSet::new(),
                queue: VecDeque::new(),
                dropped: 0,
                last_used: Instant::now(),
            }
        });
        b.scope = scope;
        b.kinds = kinds.iter().copied().collect();
        b.filter = filter;
        // Что уже накопилось, но больше не просят или не положено, выбрасываем.
        let (allowed, wanted, filter) = (b.scope.clone(), b.kinds.clone(), b.filter.clone());
        b.queue.retain(|e| {
            let kind = e["kind"].as_str().unwrap_or("");
            wanted.contains(kind) && allowed.allows(e) && (kind != "presence" || filter.matches(e))
        });
        b.last_used = Instant::now();
    }
    CHANGED.notify_all(); // насос узнаёт о новом слушателе
    loop {
        let Some(b) = guard.as_mut().and_then(|boxes| boxes.get_mut(key)) else {
            return Err("the listener was removed (the app was revoked)".into());
        };
        b.last_used = Instant::now();
        let left = deadline.saturating_duration_since(Instant::now());
        if !b.queue.is_empty() || left.is_zero() || gone() {
            let n = b.queue.len().min(max);
            let events: Vec<Value> = b.queue.drain(..n).collect();
            let dropped = std::mem::take(&mut b.dropped);
            let mut kinds: Vec<&str> = b.kinds.iter().copied().collect();
            kinds.sort_unstable();
            let problem = PROBLEM.lock().unwrap().clone();
            return Ok((
                json!({ "events": events, "dropped": dropped, "more": !b.queue.is_empty(), "kinds": kinds, "problem": problem }),
                created,
            ));
        }
        guard = CHANGED.wait_timeout(guard, left.min(GONE_CHECK)).unwrap().0;
    }
}

/// Объединение видов у всех слушателей (и убираем забытые ящики). Второе значение: условия на присутствие для payload,
/// чтобы он не копил лишнее: `null`, если кому-то нужны все события, иначе список условий (подходит любое из них).
fn wanted_kinds() -> (Vec<&'static str>, Value) {
    with_hub(|boxes| {
        boxes.retain(|_, b| b.last_used.elapsed() < MAILBOX_IDLE);
        let mut all: Vec<&'static str> = vec![];
        let mut filters: Vec<Value> = vec![];
        let mut everything = false;
        for b in boxes.values() {
            for k in &b.kinds {
                if !all.contains(k) {
                    all.push(k);
                }
            }
            if b.kinds.contains("presence") {
                if b.filter.is_empty() {
                    everything = true;
                } else if !filters.contains(&b.filter.to_json()) {
                    filters.push(b.filter.to_json());
                }
            }
        }
        all.sort_unstable();
        (all, if everything || filters.is_empty() { Value::Null } else { Value::Array(filters) })
    })
}

/// Фоновый поток: пока есть слушатели, держит подписку в payload и забирает события.
pub fn start() {
    thread::spawn(|| {
        let mut set: Vec<&'static str> = vec![];
        let mut set_presence = Value::Null;
        let mut last_set = Instant::now();
        loop {
            let (kinds, presence) = wanted_kinds();
            if kinds.is_empty() {
                if !set.is_empty() {
                    let _ = hook::call("watch_set", json!({ "kinds": [] }));
                    set.clear();
                }
                // Спим, пока не появится слушатель.
                let g = HUB.lock().unwrap();
                let _ = CHANGED.wait_timeout(g, Duration::from_secs(2));
                continue;
            }
            if kinds != set || presence != set_presence || last_set.elapsed() > RESET_EVERY {
                match hook::call("watch_set", json!({ "kinds": kinds, "presence": presence })) {
                    Ok(_) => {
                        *PROBLEM.lock().unwrap() = None;
                        if kinds != set {
                            log(&format!("watching {}", kinds.join(", ")));
                        }
                        set = kinds.clone();
                        set_presence = presence.clone();
                        last_set = Instant::now();
                    }
                    Err(e) => {
                        // Discord ещё не готов, закрыт или не нашёлся диспетчер событий: попробуем позже, а причину
                        // отдаём слушателям, иначе тишина выглядела бы как «ничего не происходит».
                        let mut problem = PROBLEM.lock().unwrap();
                        if problem.as_deref() != Some(e.as_str()) {
                            log(&format!("cannot watch: {e}"));
                            *problem = Some(e);
                        }
                        drop(problem);
                        set.clear();
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                }
            }
            match hook::call("watch_drain", json!({})) {
                Ok(v) => {
                    let events = v["events"].as_array().cloned().unwrap_or_default();
                    let lost = v["dropped"].as_u64().unwrap_or(0);
                    if lost > 0 {
                        with_hub(|boxes| boxes.values_mut().for_each(|b| b.dropped += lost));
                    }
                    // Страница перезагрузилась: подписка в ней пропала, ставим заново сразу.
                    let watching = v["watching"].as_array().map_or(0, |a| a.len());
                    if watching != kinds.len() {
                        set.clear();
                    }
                    publish(events);
                }
                Err(_) => set.clear(),
            }
            thread::sleep(POLL_EVERY);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_perms(read: &[&str]) -> Permissions {
        Permissions { read: read.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    fn msg(channel: &str, guild: Option<&str>) -> Value {
        let kind = if guild.is_some() { "text" } else { "dm" };
        json!({ "kind": "message", "channel_id": channel, "guild_id": guild, "channel": { "kind": kind }, "message": { "content": "hi" } })
    }

    const A: &str = "111111111111111111";
    const B: &str = "222222222222222222";
    const G: &str = "333333333333333333";
    const NONE: &dyn Fn() -> bool = &|| false;

    /// Хаб один на весь процесс, а тесты идут параллельно: по очереди, иначе события одного попадут в ящики другого.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn presence(user: &str, status: &str, prev: Option<&str>, changes: &[(&str, &str)]) -> Value {
        json!({
            "kind": "presence", "channel_id": null, "guild_id": null,
            "user": { "id": user }, "status": status, "prev_status": prev,
            "changes": changes.iter().map(|(c, n)| json!({ "change": c, "type": "playing", "name": n })).collect::<Vec<_>>()
        })
    }

    #[test]
    fn presence_filters_parse_strictly() {
        let f =
            PresenceFilter::parse(&json!({ "status": ["Online"], "activity": "Minecraft", "users": ["42"] })).unwrap();
        assert_eq!(f.status, vec!["online"]);
        assert_eq!(f.activity, vec!["minecraft"], "a single word instead of a list, lowercased");
        assert_eq!(f.users, vec!["42"], "user ids keep their case");
        assert!(PresenceFilter::parse(&Value::Null).unwrap().is_empty());
        assert!(PresenceFilter::parse(&json!({ "status": ["away"] })).is_err(), "not a Discord status");
        assert!(PresenceFilter::parse(&json!({ "gamez": [] })).is_err(), "a typo must not silently match everything");
        assert!(PresenceFilter::parse(&json!({ "activity": [1] })).is_err());
        assert!(PresenceFilter::parse(&json!({ "activity": [""] })).is_err());
        assert!(PresenceFilter::parse(&json!("online")).is_err());
        let many: Vec<String> = (0..21).map(|i| format!("g{i}")).collect();
        assert!(PresenceFilter::parse(&json!({ "activity": many })).is_err());
    }

    #[test]
    fn presence_filters_match_entering_a_status_or_starting_a_game() {
        let started = presence("1", "online", Some("online"), &[("started", "Minecraft Launcher")]);
        let stopped = presence("1", "online", Some("online"), &[("stopped", "Minecraft")]);
        let came_online = presence("2", "online", Some("offline"), &[]);
        let still_online = presence("2", "online", Some("online"), &[("started", "Chess")]);
        let game = PresenceFilter { activity: vec!["minecraft".into()], ..Default::default() };
        assert!(game.matches(&started) && game.matches(&stopped), "starting or stopping counts, by part of the name");
        assert!(!game.matches(&came_online) && !game.matches(&still_online));
        let online = PresenceFilter { status: vec!["online".into()], ..Default::default() };
        assert!(online.matches(&came_online), "entered online");
        assert!(!online.matches(&still_online), "was already online: not a change");
        let both =
            PresenceFilter { status: vec!["online".into()], activity: vec!["chess".into()], ..Default::default() };
        assert!(!both.matches(&came_online) && !both.matches(&still_online), "all given fields must match");
        let only_two = PresenceFilter { users: vec!["2".into()], ..Default::default() };
        assert!(only_two.matches(&came_online) && !only_two.matches(&started));
        assert!(PresenceFilter::default().matches(&started), "no conditions means everything");
    }

    #[test]
    fn presence_is_opt_in_and_needs_its_own_word() {
        let _serial = serial();
        let key = "app:test-presence-1";
        let scope =
            Scope::App(Permissions { list: vec!["presence".into()], read: vec!["*".into()], ..Default::default() });
        let game = PresenceFilter { activity: vec!["minecraft".into()], ..Default::default() };
        wait(key, scope.clone(), vec!["presence"], game.clone(), Duration::from_millis(10), 10, NONE).unwrap();
        publish(vec![
            presence("1", "online", Some("online"), &[("started", "Minecraft")]),
            presence("1", "online", Some("online"), &[("started", "Chess")]),
            msg(A, None),
        ]);
        let (v, _) = wait(key, scope, vec!["presence"], game.clone(), Duration::from_millis(10), 10, NONE).unwrap();
        let events = v["events"].as_array().unwrap();
        assert_eq!(events.len(), 1, "only the matching presence, no messages: {events:?}");
        // Без слова `presence` в list присутствие приложению не положено, даже с правом читать.
        let reader = Scope::App(read_perms(&["*"]));
        assert!(reader.filter_kinds(vec!["presence"]).is_empty());
        assert!(!reader.allows(&presence("1", "online", None, &[])));
        // Условия для payload: общий ящик без условий отменяет сужение.
        let (_, payload_filter) = wanted_kinds();
        assert_eq!(payload_filter, json!([game.to_json()]));
        wait(
            "owner:test-presence-2",
            Scope::Owner,
            vec!["presence"],
            PresenceFilter::default(),
            Duration::from_millis(10),
            10,
            NONE,
        )
        .unwrap();
        assert!(wanted_kinds().1.is_null(), "somebody wants every presence event");
        drop_mailbox(key);
        drop_mailbox("owner:test-presence-2");
    }

    #[test]
    fn kind_groups_expand_and_typos_are_errors() {
        let _serial = serial();
        assert_eq!(expand(&[]).unwrap().len(), DEFAULT_KINDS.len());
        assert!(!expand(&["all".into()]).unwrap().contains(&"presence"), "presence is never part of all");
        assert_eq!(expand(&["presence".into()]).unwrap(), vec!["presence"]);
        assert_eq!(expand(&["messages".into()]).unwrap(), MESSAGE_KINDS.to_vec());
        assert_eq!(expand(&["voice".into(), "voice_join".into()]).unwrap(), VOICE_KINDS.to_vec());
        assert!(expand(&["mesages".into()]).is_err());
    }

    #[test]
    fn an_app_gets_only_channels_it_may_read_and_voice_only_with_the_word() {
        let _serial = serial();
        let scope = Scope::App(read_perms(&[&format!("dm:{A}")]));
        assert!(scope.allows(&msg(A, None)));
        assert!(!scope.allows(&msg(B, None)), "another chat");
        assert!(!scope.allows(&msg(A, Some(G))), "same channel id under a guild is another resource");
        assert!(!scope.allows(&json!({ "kind": "voice_join", "channel_id": A })), "no `voices` word");
        assert!(!scope.allows(&json!({ "kind": "typing" })), "no channel, no event");
        // Канал сервера без `guild_id` в событии не должен сойти за личный чат под `dm:*`.
        let all_dms = Scope::App(read_perms(&["dm:*"]));
        let typing_in_guild = json!({ "kind": "typing", "channel_id": A, "guild_id": null, "channel": { "kind": "text", "guild_id": G } });
        assert!(!all_dms.allows(&typing_in_guild));
        assert!(
            !all_dms.allows(&json!({ "kind": "typing", "channel_id": A, "guild_id": null })),
            "no proof it is a DM"
        );
        let guild_scope = Scope::App(read_perms(&[&format!("guild:{G}")]));
        assert!(guild_scope.allows(&typing_in_guild), "the guild comes from the channel description");
        assert!(all_dms.allows(&json!({ "kind": "typing", "channel_id": A, "channel": { "kind": "group" } })));
        let voices = Scope::App(Permissions { list: vec!["voices".into()], ..Default::default() });
        assert!(voices.allows(&json!({ "kind": "voice_leave", "channel_id": B })));
        assert!(!voices.allows(&msg(A, None)), "voices does not open messages");
        assert!(Scope::Owner.allows(&msg(B, None)));
    }

    #[test]
    fn kinds_are_cut_down_to_what_the_rights_allow() {
        let _serial = serial();
        let reader = Scope::App(read_perms(&["*"]));
        assert_eq!(reader.filter_kinds(ALL_KINDS.to_vec()).len(), 4, "messages and typing, no voice");
        let nothing = Scope::App(Permissions::default());
        assert!(nothing.filter_kinds(ALL_KINDS.to_vec()).is_empty());
        assert_eq!(Scope::Owner.filter_kinds(ALL_KINDS.to_vec()).len(), 9);
        let watcher = Scope::App(Permissions { list: vec!["presence".into()], ..Default::default() });
        assert_eq!(watcher.filter_kinds(ALL_KINDS.to_vec()), vec!["presence"]);
    }

    #[test]
    fn events_wait_in_the_mailbox_between_calls_and_respect_rights() {
        let _serial = serial();
        let key = "app:test-mailbox-1";
        let scope = Scope::App(read_perms(&[&format!("dm:{A}")]));
        // Первый вызов создаёт ящик и ничего не находит.
        let (v, created) =
            wait(key, scope.clone(), vec!["message"], PresenceFilter::default(), Duration::from_millis(50), 10, NONE)
                .unwrap();
        assert!(created && v["events"].as_array().unwrap().is_empty());
        publish(vec![msg(A, None), msg(B, None), json!({ "kind": "typing", "channel_id": A })]);
        let (v, created) =
            wait(key, scope.clone(), vec!["message"], PresenceFilter::default(), Duration::from_millis(50), 10, NONE)
                .unwrap();
        assert!(!created);
        let events = v["events"].as_array().unwrap();
        assert_eq!(events.len(), 1, "only the allowed channel and the asked kind: {events:?}");
        assert_eq!(events[0]["channel_id"], A);
        drop_mailbox(key);
        assert!(
            wait(key, scope, vec!["message"], PresenceFilter::default(), Duration::from_millis(10), 10, NONE)
                .unwrap()
                .1,
            "recreated after a drop"
        );
        drop_mailbox(key);
    }

    #[test]
    fn a_waiting_call_returns_as_soon_as_an_event_arrives() {
        let _serial = serial();
        let key = "app:test-mailbox-2";
        wait(key, Scope::Owner, vec!["message"], PresenceFilter::default(), Duration::from_millis(10), 10, NONE)
            .unwrap();
        let t = thread::spawn(|| {
            thread::sleep(Duration::from_millis(200));
            publish(vec![msg(A, None)]);
        });
        let started = Instant::now();
        let (v, _) =
            wait(key, Scope::Owner, vec!["message"], PresenceFilter::default(), Duration::from_secs(10), 10, NONE)
                .unwrap();
        assert_eq!(v["events"].as_array().unwrap().len(), 1);
        assert!(started.elapsed() < Duration::from_secs(3), "must not wait for the timeout");
        t.join().unwrap();
        drop_mailbox(key);
    }

    #[test]
    fn a_full_mailbox_drops_the_oldest_and_says_so() {
        let _serial = serial();
        let key = "app:test-mailbox-3";
        wait(key, Scope::Owner, vec!["message"], PresenceFilter::default(), Duration::from_millis(10), 1000, NONE)
            .unwrap();
        publish((0..MAILBOX_MAX + 5).map(|i| json!({ "kind": "message", "channel_id": A, "n": i })).collect());
        let (v, _) =
            wait(key, Scope::Owner, vec!["message"], PresenceFilter::default(), Duration::from_millis(10), 1000, NONE)
                .unwrap();
        assert_eq!(v["events"].as_array().unwrap().len(), MAILBOX_MAX);
        assert_eq!(v["dropped"], 5);
        assert_eq!(v["events"][0]["n"], 5, "the oldest went first");
        drop_mailbox(key);
    }

    #[test]
    fn max_limits_one_answer_and_reports_more() {
        let _serial = serial();
        let key = "app:test-mailbox-4";
        wait(key, Scope::Owner, vec!["message"], PresenceFilter::default(), Duration::from_millis(10), 2, NONE)
            .unwrap();
        publish(vec![msg(A, None), msg(A, None), msg(A, None)]);
        let (v, _) =
            wait(key, Scope::Owner, vec!["message"], PresenceFilter::default(), Duration::from_millis(10), 2, NONE)
                .unwrap();
        assert_eq!(v["events"].as_array().unwrap().len(), 2);
        assert_eq!(v["more"], true);
        drop_mailbox(key);
    }

    #[test]
    fn a_revoked_listener_is_told_and_a_hung_up_client_returns_early() {
        let _serial = serial();
        let key = "app:test-mailbox-5";
        wait(key, Scope::Owner, vec!["message"], PresenceFilter::default(), Duration::from_millis(10), 1, NONE)
            .unwrap();
        let t = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            drop_mailbox(key);
        });
        let r = wait(key, Scope::Owner, vec!["message"], PresenceFilter::default(), Duration::from_secs(10), 1, NONE);
        // Ящик пересоздаётся при входе в `wait`, поэтому отзыв во время ожидания виден как ошибка или как пустой ответ.
        assert!(r.is_err() || r.unwrap().0["events"].as_array().unwrap().is_empty());
        t.join().unwrap();
        let key = "app:test-mailbox-6";
        let started = Instant::now();
        wait(key, Scope::Owner, vec!["message"], PresenceFilter::default(), Duration::from_secs(10), 1, &|| true)
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(2), "a client that hung up must not hold the call");
        drop_mailbox(key);
    }
}
