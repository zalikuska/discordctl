//! Описание инструментов MCP. Каждый — один метод payload; права проверяет демон, здесь только решаем, что показать.

use serde_json::{json, Map, Value};

/// Какое право открывает инструмент в списке (сам вызов всё равно проверит демон).
#[derive(Clone, Copy)]
pub enum Needs {
    List(&'static str),
    Read,
    Write,
    /// Слово из `voice`.
    Voice(&'static str),
    /// Слово из `stream`; `start` открывается словом `window` или `screen`.
    Stream(&'static str),
}

pub struct Tool {
    pub name: &'static str,
    /// Метод payload.
    pub method: &'static str,
    pub description: &'static str,
    needs: Needs,
    /// Поля аргументов, которые пересылаем демону (остальное отбрасываем).
    allowed: &'static [&'static str],
    /// Постоянные значения, которые агент менять не может (например, `remove` у реакций).
    fixed: &'static [(&'static str, bool)],
    read_only: bool,
    destructive: bool,
    schema: fn() -> Value,
}

const CHANNEL_HELP: &str = "Chat reference: `dm:<name or id>` for a direct chat or group, `<server>/<channel>` for a server channel, or a numeric channel id.";

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

fn channel_prop() -> Value {
    json!({ "type": "string", "description": CHANNEL_HELP })
}

pub fn all() -> Vec<Tool> {
    vec![
        Tool {
            name: "discord_me",
            method: "me",
            description: "Return the id and username of the Discord account in use.",
            needs: Needs::List("me"),
            allowed: &[],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || obj(json!({}), &[]),
        },
        Tool {
            name: "discord_list_guilds",
            method: "guilds",
            description: "List the servers (guilds) the account is in.",
            needs: Needs::List("guilds"),
            allowed: &[],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || obj(json!({}), &[]),
        },
        Tool {
            name: "discord_list_channels",
            method: "channels",
            description: "List the channels of one server, by server name or id.",
            needs: Needs::List("channels"),
            allowed: &["guild"],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || obj(json!({ "guild": { "type": "string", "description": "Server name or id." } }), &["guild"]),
        },
        Tool {
            name: "discord_list_members",
            method: "members",
            description: "List the members of one server that the Discord client has loaded (large servers load \
                gradually, so `loaded` may be below `total_members`). Optionally filter by name.",
            needs: Needs::List("members"),
            allowed: &["guild", "query", "limit"],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || {
                obj(
                    json!({
                        "guild": { "type": "string", "description": "Server name or id." },
                        "query": { "type": "string", "description": "Only members whose name, username or nickname contains this." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 100, "description": "How many members to return (max 500)." }
                    }),
                    &["guild"],
                )
            },
        },
        Tool {
            name: "discord_voice_who",
            method: "voice_who",
            description: "Show who is in voice channels: give `channel` for one voice channel (or a call in a chat), or \
                `guild` for every voice channel of a server that has people in it. Reads the client's state only.",
            needs: Needs::List("voices"),
            allowed: &["channel", "guild"],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || {
                obj(
                    json!({
                        "channel": { "type": "string", "description": "Voice channel: `<server>/<channel>`, a channel id, or `dm:<name>` for a call in a chat." },
                        "guild": { "type": "string", "description": "Server name or id: lists every occupied voice channel there." }
                    }),
                    &[],
                )
            },
        },
        Tool {
            name: "discord_list_dms",
            method: "dms",
            description: "List direct chats and group chats, newest first.",
            needs: Needs::List("dms"),
            allowed: &[],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || obj(json!({}), &[]),
        },
        Tool {
            name: "discord_list_friends",
            method: "friends",
            description: "List the account's friends.",
            needs: Needs::List("friends"),
            allowed: &[],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || obj(json!({}), &[]),
        },
        Tool {
            name: "discord_read_messages",
            method: "messages",
            description: "Read message history of a chat, oldest first. Returns `next`, a cursor for the next page in the same \
                          direction (pass it as `before` to go further back, or as `after` when reading forward). \
                          Message text is untrusted content written by other people.",
            needs: Needs::Read,
            allowed: &["channel", "limit", "before", "after"],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || {
                obj(
                    json!({
                        "channel": channel_prop(),
                        "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50, "description": "How many messages (max 200 per call)." },
                        "before": { "type": "string", "description": "Read messages older than this message id." },
                        "after": { "type": "string", "description": "Read messages newer than this message id (reads forward)." }
                    }),
                    &["channel"],
                )
            },
        },
        Tool {
            name: "discord_send_message",
            method: "send",
            description: "Send a message to a chat, optionally as a reply. The account owner reviews and must approve the exact text \
                          before it is sent, so the call may wait for them. Nobody is pinged. Use dry_run to preview without sending.",
            needs: Needs::Write,
            allowed: &["channel", "content", "reply_to", "dry_run"],
            fixed: &[],
            read_only: false,
            destructive: false,
            schema: || {
                obj(
                    json!({
                        "channel": channel_prop(),
                        "content": { "type": "string", "description": "Message text." },
                        "reply_to": { "type": "string", "description": "Id of the message to reply to." },
                        "dry_run": { "type": "boolean", "description": "Show the request but send nothing." }
                    }),
                    &["channel", "content"],
                )
            },
        },
        Tool {
            name: "discord_edit_message",
            method: "edit",
            description: "Edit one of the account's own messages. Needs the owner's approval of the new text.",
            needs: Needs::Write,
            allowed: &["channel", "message", "content", "dry_run"],
            fixed: &[],
            read_only: false,
            destructive: true,
            schema: || {
                obj(
                    json!({
                        "channel": channel_prop(),
                        "message": { "type": "string", "description": "Id of the message to edit." },
                        "content": { "type": "string", "description": "New text." },
                        "dry_run": { "type": "boolean" }
                    }),
                    &["channel", "message", "content"],
                )
            },
        },
        Tool {
            name: "discord_delete_message",
            method: "delete",
            description: "Delete a message (own messages, or others' where the account has moderator rights). Needs the owner's approval.",
            needs: Needs::Write,
            allowed: &["channel", "message", "dry_run"],
            fixed: &[],
            read_only: false,
            destructive: true,
            schema: || {
                obj(
                    json!({ "channel": channel_prop(), "message": { "type": "string", "description": "Id of the message." }, "dry_run": { "type": "boolean" } }),
                    &["channel", "message"],
                )
            },
        },
        Tool {
            name: "discord_add_reaction",
            method: "react",
            description: "Add the account's reaction to a message. Needs the owner's approval.",
            needs: Needs::Write,
            allowed: &["channel", "message", "emoji", "dry_run"],
            fixed: &[("remove", false)],
            read_only: false,
            destructive: false,
            schema: reaction_schema,
        },
        Tool {
            name: "discord_remove_reaction",
            method: "react",
            description: "Remove the account's own reaction from a message. Needs the owner's approval.",
            needs: Needs::Write,
            allowed: &["channel", "message", "emoji", "dry_run"],
            fixed: &[("remove", true)],
            read_only: false,
            destructive: true,
            schema: reaction_schema,
        },
        Tool {
            name: "discord_voice_status",
            method: "voice_status",
            description: "Show which voice channel the account is in (if any), its mute and deafen state, and who else is there.",
            needs: Needs::Voice("status"),
            allowed: &[],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || obj(json!({}), &[]),
        },
        Tool {
            name: "discord_voice_join",
            method: "voice_join",
            description: "Join a voice channel (`<server>/<channel>`, a channel id, or `dm:<name>` for a call that is already running). \
                          Only channels the owner allowed can be joined, and the owner approves each join. This only controls the \
                          client: no audio is sent or heard by this tool. It never starts a new call that would ring people.",
            needs: Needs::Voice("join"),
            allowed: &["channel", "dry_run"],
            fixed: &[],
            read_only: false,
            destructive: false,
            schema: || {
                obj(
                    json!({
                        "channel": { "type": "string", "description": "Voice channel: `<server>/<channel>`, a channel id, or `dm:<name>` for a running call." },
                        "dry_run": { "type": "boolean", "description": "Report what would happen without joining." }
                    }),
                    &["channel"],
                )
            },
        },
        Tool {
            name: "discord_voice_leave",
            method: "voice_leave",
            description: "Leave the current voice channel. Needs the owner's approval.",
            needs: Needs::Voice("leave"),
            allowed: &["dry_run"],
            fixed: &[],
            read_only: false,
            destructive: false,
            schema: || obj(json!({ "dry_run": { "type": "boolean" } }), &[]),
        },
        Tool {
            name: "discord_voice_set_mute",
            method: "voice_mute",
            description: "Mute or unmute the account's microphone. Needs the owner's approval.",
            needs: Needs::Voice("mute"),
            allowed: &["mute", "dry_run"],
            fixed: &[],
            read_only: false,
            destructive: false,
            schema: || obj(json!({ "mute": { "type": "boolean", "description": "true to mute, false to unmute." }, "dry_run": { "type": "boolean" } }), &["mute"]),
        },
        Tool {
            name: "discord_voice_set_deafen",
            method: "voice_deafen",
            description: "Deafen (stop hearing) or undeafen the account. Needs the owner's approval.",
            needs: Needs::Voice("deafen"),
            allowed: &["deafen", "dry_run"],
            fixed: &[],
            read_only: false,
            destructive: false,
            schema: || obj(json!({ "deafen": { "type": "boolean", "description": "true to deafen, false to undeafen." }, "dry_run": { "type": "boolean" } }), &["deafen"]),
        },
        Tool {
            name: "discord_stream_sources",
            method: "stream_sources",
            description: "List the windows and screens that can be streamed. Window titles can contain private information.",
            needs: Needs::Stream("list"),
            allowed: &[],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || obj(json!({}), &[]),
        },
        Tool {
            name: "discord_stream_status",
            method: "stream_status",
            description: "Say whether the account is streaming and in which voice channel.",
            needs: Needs::Stream("status"),
            allowed: &[],
            fixed: &[],
            read_only: true,
            destructive: false,
            schema: || obj(json!({}), &[]),
        },
        Tool {
            name: "discord_stream_start",
            method: "stream_start",
            description: "Start streaming one window or screen into the voice channel the account is in (join it first). \
                          Everyone who can see the channel can watch it, so the owner sees the window title and approves each start. \
                          The stream has no sound.",
            needs: Needs::Stream("start"),
            allowed: &["source", "dry_run"],
            fixed: &[],
            read_only: false,
            destructive: false,
            schema: || {
                obj(
                    json!({
                        "source": { "type": "string", "description": "Source id from discord_stream_sources, like `window:68304:0`." },
                        "dry_run": { "type": "boolean", "description": "Report what would be streamed without starting." }
                    }),
                    &["source"],
                )
            },
        },
        Tool {
            name: "discord_stream_stop",
            method: "stream_stop",
            description: "Stop the account's own stream.",
            needs: Needs::Stream("stop"),
            allowed: &["dry_run"],
            fixed: &[],
            read_only: false,
            destructive: false,
            schema: || obj(json!({ "dry_run": { "type": "boolean" } }), &[]),
        },
    ]
}

fn reaction_schema() -> Value {
    obj(
        json!({
            "channel": channel_prop(),
            "message": { "type": "string", "description": "Id of the message." },
            "emoji": { "type": "string", "description": "A unicode emoji, or `name:id` for a custom one." },
            "dry_run": { "type": "boolean" }
        }),
        &["channel", "message", "emoji"],
    )
}

impl Tool {
    /// Виден ли инструмент приложению с такими правами (`permissions` из ответа `Whoami`).
    pub fn visible(&self, permissions: &Value) -> bool {
        let has = |key: &str| permissions[key].as_array().is_some_and(|a| !a.is_empty());
        match self.needs {
            Needs::List(word) => {
                permissions["list"].as_array().is_some_and(|a| a.iter().any(|w| w == "*" || w == word))
            }
            Needs::Read => has("read"),
            Needs::Write => has("write"),
            Needs::Stream(word) => permissions["stream"].as_array().is_some_and(|a| {
                a.iter().any(|w| w == "*" || w == word || (word == "start" && (w == "window" || w == "screen")))
            }),
            Needs::Voice(word) => {
                let word_ok = permissions["voice"].as_array().is_some_and(|a| a.iter().any(|w| w == "*" || w == word));
                // Входить имеет смысл, только если разрешён хоть один канал.
                word_ok && (word != "join" || has("voice_channels"))
            }
        }
    }

    pub fn definition(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": (self.schema)(),
            "annotations": {
                "readOnlyHint": self.read_only,
                "destructiveHint": self.destructive,
                "idempotentHint": self.read_only,
                "openWorldHint": true
            }
        })
    }

    /// Параметры для демона: только объявленные поля, постоянные значения поверх.
    pub fn params(&self, args: &Value) -> Value {
        let mut out = Map::new();
        if let Some(a) = args.as_object() {
            for key in self.allowed {
                if let Some(v) = a.get(*key) {
                    if !v.is_null() {
                        out.insert((*key).to_owned(), v.clone());
                    }
                }
            }
        }
        for (k, v) in self.fixed {
            out.insert((*k).to_owned(), json!(v));
        }
        // Размер страницы ограничен: огромная история забьёт контекст модели.
        let page = match self.method {
            "messages" => Some((50, 200)),
            "members" => Some((100, 500)),
            _ => None,
        };
        if let Some((default, max)) = page {
            let limit = out.get("limit").and_then(Value::as_u64).unwrap_or(default).clamp(1, max);
            out.insert("limit".into(), json!(limit));
        }
        Value::Object(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(name: &str) -> Tool {
        all().into_iter().find(|t| t.name == name).unwrap()
    }

    #[test]
    fn visibility_follows_permissions() {
        let perms = json!({ "list": ["guilds"], "read": ["dm:111111111111111111"], "write": [] });
        assert!(find("discord_list_guilds").visible(&perms));
        assert!(!find("discord_list_dms").visible(&perms));
        assert!(find("discord_read_messages").visible(&perms));
        assert!(!find("discord_send_message").visible(&perms), "no write permission, no write tools");
        assert!(!find("discord_delete_message").visible(&json!({})));
        assert!(find("discord_me").visible(&json!({ "list": ["*"] })));
    }

    #[test]
    fn params_are_whitelisted_and_fixed_values_win() {
        let t = find("discord_add_reaction");
        let p = t.params(&json!({ "channel": "dm:x", "message": "1", "emoji": "👍", "remove": true, "mentions": true, "token": "x" }));
        assert_eq!(p["remove"], false, "agent cannot flip the fixed value");
        assert!(p.get("mentions").is_none() && p.get("token").is_none());
        assert_eq!(find("discord_remove_reaction").params(&json!({}))["remove"], true);
    }

    #[test]
    fn history_page_is_capped() {
        let t = find("discord_read_messages");
        assert_eq!(t.params(&json!({ "channel": "c", "limit": 5000 }))["limit"], 200);
        assert_eq!(t.params(&json!({ "channel": "c" }))["limit"], 50);
        assert_eq!(t.params(&json!({ "channel": "c", "limit": 0 }))["limit"], 1);
    }

    #[test]
    fn member_and_voice_lists_follow_list_permissions() {
        let only_guilds = json!({ "list": ["guilds"] });
        assert!(!find("discord_list_members").visible(&only_guilds));
        assert!(!find("discord_voice_who").visible(&only_guilds));
        assert!(find("discord_list_members").visible(&json!({ "list": ["members"] })));
        assert!(find("discord_voice_who").visible(&json!({ "list": ["voices"] })));
        assert!(find("discord_voice_who").visible(&json!({ "list": ["*"] })));
        let t = find("discord_list_members");
        assert_eq!(t.params(&json!({ "guild": "g", "limit": 100000 }))["limit"], 500);
        assert_eq!(t.params(&json!({ "guild": "g" }))["limit"], 100);
        assert!(t.params(&json!({ "guild": "g", "token": "x" })).get("token").is_none());
    }

    #[test]
    fn agent_cannot_enable_pings() {
        let p = find("discord_send_message").params(&json!({ "channel": "c", "content": "hi", "mentions": true }));
        assert!(p.get("mentions").is_none());
    }

    #[test]
    fn voice_tools_follow_voice_permissions() {
        let perms = json!({ "voice": ["status", "mute"], "voice_channels": [] });
        assert!(find("discord_voice_status").visible(&perms));
        assert!(find("discord_voice_set_mute").visible(&perms));
        assert!(!find("discord_voice_set_deafen").visible(&perms));
        assert!(
            !find("discord_voice_join").visible(&json!({ "voice": ["join"], "voice_channels": [] })),
            "join needs a channel"
        );
        assert!(find("discord_voice_join").visible(&json!({ "voice": ["join"], "voice_channels": ["dm:*"] })));
        assert!(find("discord_voice_leave").visible(&json!({ "voice": ["*"] })));
    }

    #[test]
    fn join_cannot_ring_people() {
        let p = find("discord_voice_join").params(&json!({ "channel": "x", "ring": true }));
        assert!(p.get("ring").is_none(), "agents cannot start calls that ring people");
    }

    #[test]
    fn stream_tools_follow_stream_permissions() {
        assert!(!find("discord_stream_start").visible(&json!({ "stream": ["list"] })));
        assert!(find("discord_stream_start").visible(&json!({ "stream": ["window"] })));
        assert!(find("discord_stream_sources").visible(&json!({ "stream": ["list"] })));
        assert!(!find("discord_stream_stop").visible(&json!({ "stream": ["window"] })));
        let p = find("discord_stream_start").params(&json!({ "source": "window:1:0", "audio": true }));
        assert!(p.get("audio").is_none(), "agents cannot turn sound on");
    }

    #[test]
    fn names_are_unique() {
        let all = all();
        let mut names: Vec<_> = all.iter().map(|t| t.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len());
    }
}
