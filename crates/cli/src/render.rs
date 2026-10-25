//! Человекочитаемый вывод читающих и пишущих команд. Машинный вывод (`--json`) печатает ответ payload как есть.

use crate::ui;
use console::style;
use serde_json::Value;

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

fn rows_of(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

pub fn servers(v: &Value) {
    let rows: Vec<Vec<String>> = rows_of(v)
        .map(|g| {
            vec![
                ui::bold(s(g, "name")),
                if g["owner"] == true { ui::yellow("owner") } else { String::new() },
                ui::dim(s(g, "id")),
            ]
        })
        .collect();
    if rows.is_empty() {
        return ui::info("you are not in any server");
    }
    println!("{}", ui::table(&["SERVER", "", "ID"], &rows));
}

pub fn channels(v: &Value) {
    let rows: Vec<Vec<String>> = rows_of(v)
        .map(|c| {
            let kind = match s(c, "kind") {
                "voice" | "stage" => ui::cyan(s(c, "kind")),
                k => k.to_owned(),
            };
            vec![ui::bold(s(c, "name")), kind, ui::dim(c["category"].as_str().unwrap_or("")), ui::dim(s(c, "id"))]
        })
        .collect();
    if rows.is_empty() {
        return ui::info("no channels");
    }
    println!("{}", ui::table(&["CHANNEL", "KIND", "CATEGORY", "ID"], &rows));
}

pub fn chats(v: &Value) {
    let rows: Vec<Vec<String>> = rows_of(v)
        .map(|d| {
            let who: Vec<&str> = rows_of(&d["recipients"]).map(|u| u["username"].as_str().unwrap_or("?")).collect();
            let kind = if s(d, "kind") == "group" { ui::cyan("group") } else { "chat".to_owned() };
            vec![
                ui::bold(format!("@{}", s(d, "name"))),
                kind,
                ui::dim(ellipsize(&who.join(", "), 36)),
                ui::dim(s(d, "id")),
            ]
        })
        .collect();
    if rows.is_empty() {
        return ui::info("no chats");
    }
    println!("{}", ui::table(&["CHAT", "KIND", "WITH", "ID"], &rows));
}

pub fn friends(v: &Value) {
    let rows: Vec<Vec<String>> = rows_of(v)
        .map(|u| vec![ui::bold(s(u, "display_name")), ui::dim(format!("@{}", s(u, "username"))), ui::dim(s(u, "id"))])
        .collect();
    if rows.is_empty() {
        return ui::info("no friends");
    }
    println!("{}", ui::table(&["FRIEND", "USERNAME", "ID"], &rows));
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

/// `2026-10-24T22:35:01.123000+00:00` → `2026-10-24 22:35` (UTC).
fn stamp(t: &str) -> String {
    t.get(..16).unwrap_or(t).replace('T', " ")
}

/// Цвет автора стабилен для одного имени, чтобы в длинной переписке реплики различались.
fn author_style(name: &str) -> console::Style {
    const COLORS: [console::Color; 6] = [
        console::Color::Cyan,
        console::Color::Green,
        console::Color::Yellow,
        console::Color::Magenta,
        console::Color::Blue,
        console::Color::Red,
    ];
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    console::Style::new().fg(COLORS[(h as usize) % COLORS.len()]).bold()
}

pub fn messages(v: &Value) {
    let ch = &v["channel"];
    let title = if s(ch, "kind") == "group" || s(ch, "kind") == "dm" {
        format!("@{}", s(ch, "name"))
    } else {
        s(ch, "name").to_owned()
    };
    println!("{} {}", style(title).bold(), ui::dim(format!("({}, times in UTC)", s(ch, "kind"))));
    let msgs = rows_of(&v["messages"]).collect::<Vec<_>>();
    if msgs.is_empty() {
        ui::info("no messages");
    }
    for m in msgs {
        let who = m["author"]["display_name"].as_str().unwrap_or("?");
        let mut head = format!("{} {}", ui::dim(stamp(s(m, "timestamp"))), author_style(who).apply_to(who));
        if m["edited_timestamp"].is_string() {
            head.push_str(&ui::dim(" (edited)"));
        }
        if m["reply_to"].is_object() {
            head.push_str(&ui::dim(format!(" ↪ {}", m["reply_to"]["author"]["display_name"].as_str().unwrap_or("?"))));
        }
        let text = s(m, "content").replace('\n', "\n    ");
        println!("{head}\n    {text}");
        for a in rows_of(&m["attachments"]) {
            println!("    {} {} {}", ui::cyan("▣"), s(a, "filename"), ui::dim(format!("({} bytes)", a["size"])));
        }
        for e in rows_of(&m["embeds"]) {
            if let Some(t) = e["title"].as_str().or(e["url"].as_str()) {
                println!("    {} {}", ui::cyan("⧉"), t);
            }
        }
        let reactions: Vec<String> =
            rows_of(&m["reactions"]).map(|r| format!("{} {}", s(r, "emoji"), r["count"])).collect();
        if !reactions.is_empty() {
            println!("    {}", ui::dim(reactions.join("  ")));
        }
    }
    if v["has_more"] == true {
        let flag = if v["direction"] == "after" { "--after" } else { "--before" };
        println!("\n{}", ui::dim(format!("more available: add {flag} {}", v["next"].as_str().unwrap_or(""))));
    }
}

/// Итог записи: что сделано и где, либо запрос при `--dry-run`.
pub fn write_result(v: &Value) {
    let ch = &v["channel"];
    let place = format!("{} {}", s(ch, "kind"), style(s(ch, "name")).bold());
    if v["dry_run"] == true {
        let req = &v["request"];
        ui::warn(format!("dry run, nothing sent: {} {} in {place}", s(req, "method"), ui::dim(s(req, "url"))));
        if !req["body"].is_null() {
            println!("{}", ui::dim(serde_json::to_string_pretty(&req["body"]).unwrap_or_default()));
        }
        return;
    }
    let id = v["message"]["id"].as_str().or(v["message_id"].as_str()).unwrap_or("?");
    let what = if v["sent"] == true {
        "sent"
    } else if v["edited"] == true {
        "edited"
    } else if v["deleted"] == true {
        "deleted"
    } else if v["reacted"] == true {
        "reaction added to"
    } else {
        "reaction removed from"
    };
    ui::ok(format!("{what} message {} in {place}", ui::dim(id)));
}

/// Состояние голоса и итог действий с ним.
/// Статус цветом: в сети, неактивен, не беспокоить, не в сети.
fn status_word(status: &str) -> String {
    let text = crate::i18n::tr(status).into_owned();
    match status {
        "online" => ui::green(text),
        "idle" => ui::yellow(text),
        "dnd" => ui::red(text),
        _ => ui::dim(text),
    }
}

/// Присутствие: кто в сети и чем занят.
pub fn presence(v: &Value) {
    let rows: Vec<Vec<String>> = rows_of(&v["presences"])
        .map(|p| {
            let name = p["user"]["display_name"].as_str().or(p["user"]["username"].as_str()).unwrap_or("?");
            let doing: Vec<String> = rows_of(&p["activities"])
                .map(|a| {
                    let mut t = format!("{} {}", crate::i18n::tr(s(a, "type")), s(a, "name"));
                    if let Some(st) = a["state"].as_str().or(a["details"].as_str()).filter(|x| !x.is_empty()) {
                        t.push_str(&format!(" ({st})"));
                    }
                    t
                })
                .collect();
            vec![ui::bold(name), status_word(s(p, "status")), doing.join("; ")]
        })
        .collect();
    if rows.is_empty() {
        return ui::info("nobody matches");
    }
    println!("{}", ui::table(&["PERSON", "STATUS", "DOING"], &rows));
    ui::hint(format!("{} of {} known ({})", v["matched"], v["known"], s(v, "scope")));
    if v["truncated"] == true {
        ui::hint("more match: raise -n or narrow the filter");
    }
}

/// Время суток (UTC) из миллисекунд эпохи.
fn clock(ms: u64) -> String {
    let s = ms / 1000 % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Одна строка на событие из `discordctl watch`.
pub fn event(ev: &Value) {
    let time = ui::dim(clock(ev["ts"].as_u64().unwrap_or(0)));
    let place = {
        let ch = &ev["channel"];
        match (s(ch, "kind"), s(ch, "name")) {
            ("dm" | "group", n) => format!("@{n}"),
            (_, "") => ev["channel_id"].as_str().unwrap_or("?").to_owned(),
            (_, n) => format!("#{n}"),
        }
    };
    let who = ev["user"]["display_name"].as_str().or(ev["user"]["username"].as_str()).unwrap_or("?");
    let who = author_style(who).apply_to(who);
    // Слова фраз переводим по отдельности, а строку целиком не трогаем: в ней чужой текст сообщения.
    let word = |w: &str| crate::i18n::tr(w).into_owned();
    match s(ev, "kind") {
        "presence" => {
            let status = s(ev, "status");
            if ev["prev_status"].as_str() != Some(status) {
                println!("{time} {who} {} {}", word("is now"), status_word(status));
            }
            for c in rows_of(&ev["changes"]) {
                let phrase = match (s(c, "change"), s(c, "type")) {
                    ("started", "playing") => "started playing",
                    ("started", "streaming") => "started streaming",
                    ("started", "listening") => "started listening to",
                    ("started", "watching") => "started watching",
                    ("started", "competing") => "started competing in",
                    ("started", _) => "set a custom status",
                    ("stopped", "playing") => "stopped playing",
                    ("stopped", "streaming") => "stopped streaming",
                    ("stopped", "listening") => "stopped listening to",
                    ("stopped", "watching") => "stopped watching",
                    ("stopped", "competing") => "stopped competing in",
                    _ => "cleared the custom status",
                };
                println!("{time} {who} {} {}", word(phrase), ui::bold(s(c, "name")));
            }
        }
        "message" => {
            let m = &ev["message"];
            let mut text = s(m, "content").replace('\n', " ⏎ ");
            let files = rows_of(&m["attachments"]).count();
            if files > 0 {
                text.push_str(&word(&format!(" [{files} file(s)]")));
            }
            println!("{time} {} {who}: {text}", ui::dim(place));
        }
        "message_edit" => {
            println!(
                "{time} {} {} {} {}: {}",
                ui::dim(place),
                word("message"),
                s(ev, "message_id"),
                word("edited"),
                s(ev, "content")
            );
        }
        "message_delete" => {
            println!("{time} {} {} {} {}", ui::dim(place), word("message"), s(ev, "message_id"), word("deleted"));
        }
        "voice_join" => println!("{time} {who} {} {}", word("joined"), ui::bold(place)),
        "voice_leave" => println!("{time} {who} {} {}", word("left"), ui::bold(place)),
        "voice_move" => {
            let from = s(&ev["from_channel"], "name");
            let from = if from.is_empty() { s(ev, "from_channel_id") } else { from };
            println!("{time} {who} {} {from} {} {}", word("moved from"), word("to"), ui::bold(place));
        }
        "voice_state" => {
            let flags: Vec<String> = s(ev, "flags")
                .split(',')
                .filter(|f| !f.is_empty())
                .map(|f| match f {
                    "muted" => word("mic off"),
                    "deafened" => word("sound off"),
                    "server-muted" => word("muted by server"),
                    other => word(other),
                })
                .collect();
            println!("{time} {who} {} {}: {}", word("in"), ui::bold(place), flags.join(", "));
        }
        "typing" => println!("{time} {who} {} {}", word("is typing in"), ui::dim(place)),
        other => println!("{time} {other}"),
    }
}

pub fn voice(v: &Value) {
    if v["dry_run"] == true {
        return ui::warn(format!("dry run, nothing done: {}", serde_json::to_string(v).unwrap_or_default()));
    }
    match v["channel"].as_object() {
        Some(_) => {
            let ch = &v["channel"];
            ui::ok(format!(
                "in voice: {} {}",
                style(s(ch, "name")).bold(),
                ui::dim(format!("({}, {})", s(ch, "kind"), s(v, "state")))
            ));
        }
        None => ui::info(format!("not in a voice channel {}", ui::dim(format!("({})", s(v, "state"))))),
    }
    ui::say(format!(
        "  microphone {}   sound {}",
        if v["self_mute"] == true { ui::red("muted") } else { ui::green("on") },
        if v["self_deaf"] == true { ui::red("off") } else { ui::green("on") }
    ));
    people(&v["participants"], "  ");
}

/// Участники голосового канала: имя и пометки (микрофон, звук, трансляция, видео).
fn people(list: &Value, indent: &str) {
    for p in rows_of(list) {
        let mut flags: Vec<String> = Vec::new();
        for (key, word) in
            [("muted", "muted"), ("deafened", "deafened"), ("streaming", "streaming"), ("video", "video")]
        {
            if p[key] == true {
                flags.push(crate::i18n::tr(word).into_owned());
            }
        }
        let tail = if flags.is_empty() { String::new() } else { ui::dim(format!(" [{}]", flags.join(", "))) };
        println!("{indent}{} {}{}", ui::cyan("●"), p["user"]["display_name"].as_str().unwrap_or("?"), tail);
    }
}

/// Кто в голосовых каналах: один канал или все занятые каналы сервера.
pub fn voice_who(v: &Value) {
    if let Some(ch) = v["channel"].as_object().map(|_| &v["channel"]) {
        let n = rows_of(&v["participants"]).count();
        ui::info(format!("{} {}", style(s(ch, "name")).bold(), ui::dim(format!("({}, {n} here)", s(ch, "kind")))));
        if n == 0 {
            return ui::hint("nobody is there");
        }
        return people(&v["participants"], "  ");
    }
    let total = v["people_in_voice"].as_u64().unwrap_or(0);
    ui::info(format!(
        "{}: {total} in voice, {} voice channels",
        style(s(&v["guild"], "name")).bold(),
        v["voice_channels"].as_u64().unwrap_or(0)
    ));
    if total == 0 {
        return ui::hint("nobody is in a voice channel");
    }
    for c in rows_of(&v["channels"]) {
        println!("\n{}", style(s(&c["channel"], "name")).bold());
        people(&c["participants"], "  ");
    }
}

/// Участники сервера, которых клиент уже загрузил.
pub fn members(v: &Value) {
    let rows: Vec<Vec<String>> = rows_of(&v["members"])
        .map(|m| {
            let name = m["display_name"].as_str().unwrap_or("?");
            let nick = m["nickname"].as_str().filter(|n| *n != name).map(|n| format!(" ({n})")).unwrap_or_default();
            let roles =
                m["roles"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "));
            vec![
                ui::bold(format!("{name}{nick}")),
                if m["bot"] == true { ui::yellow("bot") } else { String::new() },
                ui::dim(s(m, "username")),
                roles.unwrap_or_default(),
            ]
        })
        .collect();
    if rows.is_empty() {
        return ui::info("no loaded members match");
    }
    println!("{}", ui::table(&["MEMBER", "", "USERNAME", "ROLES"], &rows));
    let loaded = v["loaded"].as_u64().unwrap_or(0);
    let total = v["total_members"].as_u64();
    let mut note = format!("{} of {loaded} loaded members", rows.len());
    if let Some(t) = total {
        note.push_str(&format!(" (the server has {t})"));
    }
    if v["truncated"] == true {
        note.push_str(": more match, raise -n or narrow the filter");
    }
    ui::hint(note);
}

pub fn stream_sources(v: &Value) {
    let rows: Vec<Vec<String>> =
        rows_of(v).map(|x| vec![ui::bold(s(x, "name")), s(x, "kind").to_owned(), ui::dim(s(x, "id"))]).collect();
    if rows.is_empty() {
        return ui::info("nothing to share");
    }
    println!("{}", ui::table(&["WINDOW OR SCREEN", "KIND", "ID"], &rows));
}

/// Итог команд трансляции.
pub fn stream(v: &Value) {
    let src = &v["source"];
    let what = if src.is_object() {
        format!("{} {}", s(src, "kind"), style(format!("\"{}\"", s(src, "name"))).bold())
    } else {
        String::new()
    };
    if v["dry_run"] == true {
        return if what.is_empty() {
            ui::warn(format!("dry run, nothing done (streaming: {})", v["streaming"]))
        } else {
            ui::warn(format!("dry run, nothing started: would share {what}"))
        };
    }
    let place =
        v["voice"]["name"].as_str().map(|n| format!(" in voice channel {}", style(n).bold())).unwrap_or_default();
    if v["started"] == true {
        ui::ok(format!("sharing {what}{place}"));
    } else if v["stopped"] == true {
        ui::ok("sharing stopped");
    } else if v["streaming"] == true {
        ui::ok(format!("sharing{place}"));
    } else {
        ui::info(format!("not sharing{place}"));
    }
}
