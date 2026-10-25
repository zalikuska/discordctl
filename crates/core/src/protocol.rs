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
    /// Вызвать метод payload в Discord. Без токена (владелец) прав не проверяется; с токеном демон сверяет их по методу.
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
    /// Ждать события Discord (новые сообщения, голос, набор текста). Отвечает, как только что-то пришло или вышло
    /// `wait_secs`. Только владелец; у приложений то же самое делает метод `events` через `call`.
    Events {
        /// Виды или группы (`messages`, `voice`, `all`); пусто значит всё.
        #[serde(default)]
        kinds: Vec<String>,
        #[serde(default)]
        wait_secs: u64,
        /// Не больше стольки событий за раз (0: по умолчанию).
        #[serde(default)]
        max: usize,
        /// Имя слушателя: у каждого свой ящик (по умолчанию общий).
        #[serde(default)]
        sub: Option<String>,
        /// Условия для присутствия: `{"status": [...], "activity": [...], "users": [...]}`.
        #[serde(default)]
        filter: Option<serde_json::Value>,
    },
    /// Проверки: положить события в очереди так, будто их прислал Discord. Только владелец.
    DevEmit {
        events: Vec<serde_json::Value>,
    },
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
