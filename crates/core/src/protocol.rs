//! Протокол локального API демона: одна JSON-строка запроса, одна JSON-строка ответа.

use crate::permissions::Permissions;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Status,
    /// Запустить Discord под управлением демона (или переслать аргументы уже запущенному).
    Launch {
        args: Vec<String>,
    },
    /// Остановить демон. Без `force` отказывает, пока управляемый Discord запущен:
    /// закрытие CDP-пайпа завершает Discord.
    Shutdown {
        force: bool,
    },
    /// Вызвать метод payload в Discord. Пока без проверки прав (шаг 7 плана): доступно любому процессу пользователя.
    Call {
        method: String,
        params: serde_json::Value,
    },
    /// Разработка: заменить payload (из файла или вшитый) и перезагрузить страницу Discord.
    HookReload {
        payload_path: Option<String>,
    },
    /// Кто я и что мне можно (для приложений с токеном).
    Whoami,
    /// Создать (или пересоздать) приложение с правами и выдать токен. Только владелец; человека спрашивает CLI.
    AuthGrant {
        name: String,
        permissions: Permissions,
    },
    AuthRevoke {
        name: String,
    },
    AuthList,
    /// Последние записи журнала аудита.
    AuthAudit {
        limit: usize,
    },
    /// Ожидающие подтверждения вызовы приложений.
    Approvals,
    /// Решение человека по ожидающему вызову.
    Decide {
        id: u64,
        allow: bool,
    },
}

/// Запрос с необязательным токеном приложения. Без токена запрос считается запросом владельца (CLI).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(flatten)]
    pub req: Request,
}

/// Переменная окружения, из которой любой клиент берёт токен приложения.
pub const TOKEN_ENV: &str = "DISCORDCTL_TOKEN";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Response {
    pub fn ok(msg: impl Into<String>) -> Self {
        Response { ok: true, msg: Some(msg.into()), data: None }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Response { ok: false, msg: Some(msg.into()), data: None }
    }
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
}
