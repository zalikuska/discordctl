//! Права приложений: манифест, шаблоны ресурсов и проверка.
//!
//! Права хранятся по **id**, не по именам: имя чата можно сменить или подделать, id нет.
//! Имена из манифеста превращаются в id в момент одобрения (см. CLI), при проверке имён уже нет.

use serde::{Deserialize, Serialize};

/// Что приложение может делать.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Списки: `guilds`, `channels`, `dms`, `friends`, `me`, `members` (участники сервера), `voices` (кто в голосовых
    /// каналах) или `*`.
    #[serde(default)]
    pub list: Vec<String>,
    /// Чтение истории: шаблоны ресурсов.
    #[serde(default)]
    pub read: Vec<String>,
    /// Запись: шаблоны ресурсов.
    #[serde(default)]
    pub write: Vec<String>,
    /// Голос: `join`, `mute`, `deafen`, `status` или `*` (шаг 5).
    #[serde(default)]
    pub voice: Vec<String>,
    /// В какие голосовые каналы можно входить (шаблоны ресурсов). Нужно вместе со словом `join`.
    #[serde(default)]
    pub voice_channels: Vec<String>,
    /// Трансляция: `list` (источники), `status`, `stop`, `window`, `screen` (запуск окна или всего экрана), `*`.
    #[serde(default)]
    pub stream: Vec<String>,
    /// Области, где каждый вызов подтверждает человек. Не указано: `write`, `voice`, `stream`.
    #[serde(default)]
    pub confirm: Option<Vec<String>>,
}

pub const DEFAULT_CONFIRM: [&str; 3] = ["write", "voice", "stream"];
pub const SCOPES: [&str; 5] = ["list", "read", "write", "voice", "stream"];

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub app: ManifestApp,
    #[serde(default)]
    pub permissions: Permissions,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestApp {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let mut m: Manifest = toml::from_str(text).map_err(|e| format!("manifest: {e}"))?;
        m.permissions.normalize();
        valid_app_name(&m.app.name)?;
        Ok(m)
    }
}

/// Имя попадает в журнал и в файл прав: только безопасные символы.
pub fn valid_app_name(name: &str) -> Result<(), String> {
    let ok =
        !name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("app name {name:?}: use 1-40 letters, digits, '-' or '_'"))
    }
}

/// Канал, к которому обращается вызов, в каноническом виде (после разбора на стороне Discord).
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    pub guild_id: Option<String>,
    pub channel_id: String,
}

impl Resource {
    /// Шаблон, описывающий ровно этот ресурс (для показа и записи в права).
    pub fn pattern(&self) -> String {
        match &self.guild_id {
            Some(g) => format!("guild:{g}/channel:{}", self.channel_id),
            None => format!("dm:{}", self.channel_id),
        }
    }
}

pub fn is_id(s: &str) -> bool {
    (15..=25).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// Шаблоны ресурсов:
/// `*`, `dm:*`, `dm:<id>`, `guild:<id>` (все каналы сервера), `guild:<id>/channel:*`, `guild:<id>/channel:<id>`.
pub fn valid_resource_pattern(p: &str) -> bool {
    if p == "*" || p == "dm:*" {
        return true;
    }
    if let Some(id) = p.strip_prefix("dm:") {
        return is_id(id);
    }
    if let Some(rest) = p.strip_prefix("guild:") {
        return match rest.split_once('/') {
            None => is_id(rest),
            Some((g, c)) => is_id(g) && c.strip_prefix("channel:").is_some_and(|c| c == "*" || is_id(c)),
        };
    }
    false
}

fn pattern_matches(pattern: &str, r: &Resource) -> bool {
    if pattern == "*" {
        return true;
    }
    match &r.guild_id {
        None => pattern == "dm:*" || pattern.strip_prefix("dm:") == Some(r.channel_id.as_str()),
        Some(g) => {
            let Some(rest) = pattern.strip_prefix("guild:") else { return false };
            match rest.split_once('/') {
                None => rest == g,
                Some((pg, c)) => pg == g && c.strip_prefix("channel:").is_some_and(|c| c == "*" || c == r.channel_id),
            }
        }
    }
}

impl Permissions {
    /// Привычные слова → внутренние: `servers` → `guilds`, `chats` → `dms`.
    pub fn normalize(&mut self) {
        for w in &mut self.list {
            match w.as_str() {
                "servers" => *w = "guilds".into(),
                "chats" => *w = "dms".into(),
                _ => {}
            }
        }
    }

    pub fn allows_resource(&self, scope: &str, r: &Resource) -> bool {
        let list = match scope {
            "read" => &self.read,
            "write" => &self.write,
            "voice_channels" => &self.voice_channels,
            _ => return false,
        };
        list.iter().any(|p| pattern_matches(p, r))
    }

    /// `list`, `voice`, `stream`: простые слова или `*`.
    pub fn allows_word(&self, scope: &str, word: &str) -> bool {
        let list = match scope {
            "list" => &self.list,
            "voice" => &self.voice,
            "stream" => &self.stream,
            _ => return false,
        };
        list.iter().any(|w| w == "*" || w == word)
    }

    pub fn needs_confirmation(&self, scope: &str) -> bool {
        match &self.confirm {
            Some(c) => c.iter().any(|s| s == scope),
            None => DEFAULT_CONFIRM.contains(&scope),
        }
    }

    /// Проверка формы: неизвестные слова и шаблоны — ошибка, чтобы опечатка не превратилась в молчаливый отказ или, хуже, в лишнее право.
    pub fn validate(&self) -> Result<(), String> {
        for w in &self.list {
            if !["*", "guilds", "channels", "dms", "friends", "me", "members", "voices"].contains(&w.as_str()) {
                return Err(format!("list: unknown item {w:?}"));
            }
        }
        for (scope, list) in [("read", &self.read), ("write", &self.write), ("voice_channels", &self.voice_channels)] {
            for p in list {
                if !valid_resource_pattern(p) {
                    return Err(format!(
                        "{scope}: {p:?} is not a resource pattern (dm:<id>, dm:*, guild:<id>[/channel:<id>|*], *)"
                    ));
                }
            }
        }
        for w in &self.voice {
            if !["*", "join", "leave", "mute", "deafen", "status"].contains(&w.as_str()) {
                return Err(format!("voice: unknown item {w:?} (join, leave, mute, deafen, status, *)"));
            }
        }
        if self.voice_channels.iter().any(|_| true) && !self.voice.iter().any(|w| w == "*" || w == "join") {
            return Err("voice_channels is set but voice has no \"join\"".into());
        }
        for w in &self.stream {
            if !["*", "list", "status", "stop", "window", "screen"].contains(&w.as_str()) {
                return Err(format!("stream: unknown item {w:?} (list, status, stop, window, screen, *)"));
            }
        }
        if let Some(c) = &self.confirm {
            for s in c {
                if !SCOPES.contains(&s.as_str()) {
                    return Err(format!("confirm: unknown scope {s:?}"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dm(id: &str) -> Resource {
        Resource { guild_id: None, channel_id: id.into() }
    }
    fn ch(g: &str, c: &str) -> Resource {
        Resource { guild_id: Some(g.into()), channel_id: c.into() }
    }
    const A: &str = "111111111111111111";
    const B: &str = "222222222222222222";
    const G: &str = "333333333333333333";
    const H: &str = "444444444444444444";

    fn p(read: &[&str]) -> Permissions {
        Permissions { read: read.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn dm_patterns() {
        assert!(p(&[&format!("dm:{A}")]).allows_resource("read", &dm(A)));
        assert!(!p(&[&format!("dm:{A}")]).allows_resource("read", &dm(B)));
        assert!(p(&["dm:*"]).allows_resource("read", &dm(B)));
        assert!(!p(&["dm:*"]).allows_resource("read", &ch(G, A)), "dm:* must not open server channels");
    }

    #[test]
    fn guild_patterns() {
        let all = p(&[&format!("guild:{G}")]);
        assert!(all.allows_resource("read", &ch(G, A)));
        assert!(!all.allows_resource("read", &ch(H, A)));
        assert!(!all.allows_resource("read", &dm(A)));
        let one = p(&[&format!("guild:{G}/channel:{A}")]);
        assert!(one.allows_resource("read", &ch(G, A)));
        assert!(!one.allows_resource("read", &ch(G, B)));
        assert!(!one.allows_resource("read", &ch(H, A)), "same channel id under another guild");
        assert!(p(&[&format!("guild:{G}/channel:*")]).allows_resource("read", &ch(G, B)));
    }

    #[test]
    fn scopes_are_separate() {
        let perms = p(&["*"]);
        assert!(perms.allows_resource("read", &dm(A)));
        assert!(!perms.allows_resource("write", &dm(A)), "read must not imply write");
        assert!(!Permissions::default().allows_resource("read", &dm(A)), "empty means nothing");
    }

    #[test]
    fn words() {
        let perms = Permissions { list: vec!["guilds".into()], voice: vec!["*".into()], ..Default::default() };
        assert!(perms.allows_word("list", "guilds"));
        assert!(!perms.allows_word("list", "dms"));
        assert!(perms.allows_word("voice", "join"));
        assert!(!perms.allows_word("stream", "start"));
    }

    #[test]
    fn voice_needs_word_and_channel() {
        let perms = Permissions {
            voice: vec!["join".into(), "mute".into()],
            voice_channels: vec![format!("guild:{G}/channel:{A}")],
            ..Default::default()
        };
        assert!(perms.validate().is_ok());
        assert!(perms.allows_word("voice", "join"));
        assert!(!perms.allows_word("voice", "leave"));
        assert!(perms.allows_resource("voice_channels", &ch(G, A)));
        assert!(!perms.allows_resource("voice_channels", &ch(G, B)), "only the listed voice channel");
        assert!(!perms.allows_resource("read", &ch(G, A)), "voice_channels is not a read permission");
        let no_join = Permissions {
            voice: vec!["mute".into()],
            voice_channels: vec!["guild:333333333333333333".into()],
            ..Default::default()
        };
        assert!(no_join.validate().is_err(), "channels without the join word is a mistake");
        assert!(
            Permissions { voice: vec!["jion".into()], ..Default::default() }.validate().is_err(),
            "typo in a voice word"
        );
    }

    #[test]
    fn stream_words_are_checked() {
        let ok = Permissions { stream: vec!["list".into(), "window".into()], ..Default::default() };
        assert!(ok.validate().is_ok());
        assert!(ok.allows_word("stream", "window"));
        assert!(!ok.allows_word("stream", "screen"), "window does not open the whole screen");
        assert!(Permissions { stream: vec!["start".into()], ..Default::default() }.validate().is_err(), "unknown word");
    }

    #[test]
    fn friendly_list_words_are_normalized() {
        let mut p =
            Permissions { list: vec!["servers".into(), "chats".into(), "friends".into()], ..Default::default() };
        p.normalize();
        assert_eq!(p.list, vec!["guilds", "dms", "friends"]);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn confirmation_defaults() {
        let mut perms = Permissions::default();
        assert!(perms.needs_confirmation("write"));
        assert!(!perms.needs_confirmation("read"));
        perms.confirm = Some(vec![]);
        assert!(!perms.needs_confirmation("write"), "explicit empty list turns confirmations off");
    }

    #[test]
    fn validation_rejects_names_and_typos() {
        assert!(p(&["dm:alice"]).validate().is_err(), "names must be resolved to ids first");
        assert!(p(&["guild:abc"]).validate().is_err());
        assert!(p(&[&format!("guild:{G}/chan:{A}")]).validate().is_err());
        assert!(p(&[&format!("dm:{A}"), "*"]).validate().is_ok());
        assert!(Permissions { list: vec!["guild".into()], ..Default::default() }.validate().is_err());
        assert!(Permissions { confirm: Some(vec!["writes".into()]), ..Default::default() }.validate().is_err());
    }

    #[test]
    fn manifest_parses() {
        let m = Manifest::parse(
            r#"
            [app]
            name = "my-agent"
            [permissions]
            read = ["dm:111111111111111111"]
            list = ["guilds"]
            "#,
        )
        .unwrap();
        assert_eq!(m.app.name, "my-agent");
        assert_eq!(m.permissions.read.len(), 1);
        assert!(Manifest::parse("[app]\nname = \"bad name!\"").is_err());
        assert!(
            Manifest::parse("[app]\nname = \"x\"\n[permissions]\nrede = []").is_err(),
            "a typo in a scope name must fail loudly"
        );
    }
}
