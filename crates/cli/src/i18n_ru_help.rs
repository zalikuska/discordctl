//! Русская справка (`--help`): описания команд и параметров из `cli.rs`. Ключ — текст из комментария `///` как есть.

pub const RU_HELP: &[(&str, &str)] = &[
    ("Control your own Discord client from the terminal and from AI agents", "Управляйте своим клиентом Discord из терминала и из ИИ-агентов"),
    (
        "Control your own Discord client from the terminal and from AI agents.\n\nWorks through the Chromium debugging pipe of the desktop client. Windows only.",
        "Управляйте своим клиентом Discord из терминала и из ИИ-агентов.\n\nРаботает через канал отладки Chromium у клиента для компьютера. Только Windows.",
    ),
    ("Machine-readable JSON output", "Вывод в JSON для программ"),
    ("Plain output without colors (also honors NO_COLOR)", "Вывод без цветов (учитывается и NO_COLOR)"),
    (
        "Interface language: en, ru or auto (also DISCORDCTL_LANG; `discordctl lang` saves it)",
        "Язык интерфейса: en, ru или auto (также DISCORDCTL_LANG; `discordctl lang` сохраняет выбор)",
    ),
    (
        "Install everything step by step, then offer to connect an AI agent",
        "Установить всё по шагам и предложить подключить ИИ-агента",
    ),
    (
        "Install files and the launch shim (non-interactive); the discord:// link hook is opt-in",
        "Установить файлы и лаунчер запуска (без вопросов); перехват ссылок discord:// включается отдельно",
    ),
    ("Discord install folder (default: %LOCALAPPDATA%\\Discord)", "Папка установки Discord (по умолчанию %LOCALAPPDATA%\\Discord)"),
    (
        "Also point discord:// links at the launcher (changes a registry value under HKCU)",
        "Направить ссылки discord:// на лаунчер (меняет одно значение реестра в HKCU)",
    ),
    (
        "Put Discord back the way it was (a running Discord is not closed)",
        "Вернуть Discord как было (работающий Discord не закрывается)",
    ),
    (
        "Also delete discordctl data: app tokens, audit log, logs (asks first)",
        "Удалить также данные discordctl: токены приложений, журнал, логи (спросит)",
    ),
    (
        "Check for a newer release, download it, verify it and install it",
        "Найти свежий релиз, скачать, проверить и установить",
    ),
    ("Show or change the interface language (en, ru, auto)", "Показать или сменить язык интерфейса (en, ru, auto)"),
    (
        "en, ru or auto (the Windows display language); omit to show the current one",
        "en, ru или auto (язык интерфейса Windows); без значения показывает текущий",
    ),
    ("Check the installation and say what to fix", "Проверить установку и подсказать, что чинить"),
    (
        "Repair what is safe to repair: shim, link handler, missing files",
        "Починить то, что безопасно чинить: лаунчер, обработчик ссылок, недостающие файлы",
    ),
    ("What is running: daemon, Discord, hook", "Что работает: демон, Discord, хук"),
    ("Start Discord under discordctl", "Запустить Discord через discordctl"),
    (
        "Extra arguments for Discord (only a few safe ones are allowed)",
        "Дополнительные аргументы для Discord (разрешены лишь несколько безопасных)",
    ),
    ("Servers you are in", "Ваши серверы"),
    ("Channels of a server", "Каналы сервера"),
    ("Direct chats and group chats", "Личные и групповые чаты"),
    ("Friends", "Друзья"),
    (
        "Read, send, edit, delete messages and reactions",
        "Читать, отправлять, править и удалять сообщения и реакции",
    ),
    ("Shortcut for `message read`", "Короткая форма `message read`"),
    ("Shortcut for `message send`", "Короткая форма `message send`"),
    ("Voice calls: join, leave, mute, deafen", "Голос: войти, выйти, микрофон, звук"),
    (
        "Share a window or screen into the voice channel you are in",
        "Показать окно или экран в голосовом канале, где вы сидите",
    ),
    (
        "Rights of apps and AI agents (tokens, approvals, audit log)",
        "Права приложений и ИИ-агентов (токены, одобрения, журнал)",
    ),
    (
        "Connect AI agents (Claude Code, Claude Desktop, Cursor) through MCP",
        "Подключить ИИ-агентов (Claude Code, Claude Desktop, Cursor) через MCP",
    ),
    ("The background daemon that owns the Discord connection", "Фоновый демон, который держит связь с Discord"),
    (
        "Shell completions: `discordctl completions powershell | Out-String | Invoke-Expression`",
        "Автодополнение для оболочки: `discordctl completions powershell | Out-String | Invoke-Expression`",
    ),
    (
        "Watch what happens: new messages, people joining voice, typing, who started a game (Ctrl+C to stop)",
        "Следить за событиями: новые сообщения, кто зашёл в голос, набор текста, кто начал играть (Ctrl+C: выход)",
    ),
    (
        "What to watch: messages, voice, typing, presence, all, or one kind such as voice_join (default: all but presence)",
        "За чем следить: messages, voice, typing, presence, all или один вид вроде voice_join (по умолчанию всё, кроме presence)",
    ),
    (
        "Presence: only when somebody ENTERS this status: online, idle, dnd or offline (repeatable)",
        "Присутствие: только когда кто-то ВХОДИТ в этот статус: online, idle, dnd или offline (можно повторять)",
    ),
    (
        "Presence: only when somebody starts or stops something whose name contains this, e.g. a game (repeatable)",
        "Присутствие: только когда кто-то начинает или бросает занятие с таким названием, например игру (можно повторять)",
    ),
    ("Presence: only this person (name or id, repeatable)", "Присутствие: только этот человек (имя или id, можно повторять)"),
    (
        "Who is online and what they are doing (friends, or the loaded members of a server)",
        "Кто в сети и чем занят (друзья или загруженные участники сервера)",
    ),
    ("One person (name or id) instead of everybody", "Один человек (имя или id) вместо всех"),
    (
        "Members of this server that Discord has loaded, instead of friends",
        "Участники этого сервера, которых Discord загрузил, вместо друзей",
    ),
    (
        "Only these statuses: online, idle, dnd, offline (repeatable)",
        "Только эти статусы: online, idle, dnd, offline (можно повторять)",
    ),
    (
        "Only people doing something whose name contains this, e.g. a game (repeatable)",
        "Только те, кто занят чем-то с таким названием, например игрой (можно повторять)",
    ),
    (
        "Only this chat or channel (repeatable): Server/channel, @name or an id",
        "Только этот чат или канал (можно повторять): Server/channel, @имя или id",
    ),
    ("Stop after this many events", "Остановиться после стольких событий"),
    ("Tools for working on discordctl itself", "Инструменты для разработки самого discordctl"),
    ("Anthropic Claude Code (needs the `claude` command)", "Anthropic Claude Code (нужна команда `claude`)"),
    ("Claude Desktop app", "Приложение Claude Desktop"),
    ("Cursor editor", "Редактор Cursor"),
    (
        "Do not touch any app: print the config to paste yourself",
        "Ничего не трогать: напечатать настройки, чтобы вставить самому",
    ),
    ("List things and read messages; nothing can be sent", "Списки и чтение сообщений; ничего нельзя отправить"),
    (
        "Read, plus send/edit/delete/react: every write waits for your approval",
        "Чтение плюс отправка/правка/удаление/реакции: каждая запись ждёт вашего одобрения",
    ),
    (
        "Everything (write, voice, screen sharing) with NO approval prompts: use only for an agent you fully trust",
        "Всё (запись, голос, показ экрана) БЕЗ вопросов об одобрении: только для агента, которому вы полностью доверяете",
    ),
    ("Only say whether a newer release exists", "Только сказать, есть ли новый релиз"),
    (
        "Download and verify, but do not install: unpack into the data folder and print where",
        "Скачать и проверить, но не устанавливать: распаковать в папку данных и сказать куда",
    ),
    (
        "Take this release (for example v0.1.1) instead of the latest one; older ones are allowed",
        "Взять этот релиз (например v0.1.1) вместо последнего; можно и старый",
    ),
    (
        "Do not ask (closes a running Discord if the update needs it)",
        "Не спрашивать (закрывает работающий Discord, если обновлению это нужно)",
    ),
    ("Answer yes to everything and do not ask (for scripts)", "Отвечать «да» на всё и не спрашивать (для скриптов)"),
    ("Connect this agent without asking (repeatable)", "Подключить этого агента без вопросов (можно повторять)"),
    ("Access for the agent", "Доступ для агента"),
    ("Do not offer to restart a running Discord", "Не предлагать перезапустить работающий Discord"),
    ("Use plain questions instead of the full-screen interface", "Обычные вопросы вместо полноэкранного интерфейса"),
    ("List your servers", "Список ваших серверов"),
    (
        "Members of a server that Discord has loaded (large servers load gradually)",
        "Участники сервера, которых Discord уже загрузил (большие серверы грузятся постепенно)",
    ),
    ("Server name or id", "Имя или id сервера"),
    (
        "Only members whose name, username or nickname contains this",
        "Только участники, в чьём имени, логине или нике есть этот текст",
    ),
    ("How many to show (max 500)", "Сколько показать (максимум 500)"),
    ("List the channels of a server", "Список каналов сервера"),
    ("List direct chats and groups, newest first", "Список личных чатов и групп, новые сверху"),
    ("List friends", "Список друзей"),
    (
        "How many messages (up to 2000; long reads are paced, so they take a while)",
        "Сколько сообщений (до 2000; длинное чтение идёт с паузами и занимает время)",
    ),
    ("Read messages older than this message id", "Читать сообщения старше сообщения с этим id"),
    ("Read messages newer than this message id", "Читать сообщения новее сообщения с этим id"),
    ("The message text (words are joined with spaces)", "Текст сообщения (слова соединяются пробелами)"),
    ("Reply to the message with this id", "Ответить на сообщение с этим id"),
    ("Read the text from stdin", "Взять текст из stdin"),
    (
        "Let @mentions ping people (by default nobody is pinged)",
        "Разрешить @упоминаниям пинговать людей (по умолчанию никого не пингуют)",
    ),
    ("Show the request and send nothing", "Показать запрос и ничего не отправлять"),
    ("Read message history, oldest first", "Читать историю сообщений, старые сверху"),
    ("Send a message", "Отправить сообщение"),
    ("Edit one of your messages", "Изменить своё сообщение"),
    ("Id of the message", "Id сообщения"),
    ("The new text", "Новый текст"),
    ("Delete a message", "Удалить сообщение"),
    ("Add a reaction (or remove yours)", "Добавить реакцию (или убрать свою)"),
    ("An emoji, or name:id for a custom one", "Эмодзи или имя:id для своего"),
    ("Remove your reaction instead", "Вместо этого убрать свою реакцию"),
    ("Which voice channel you are in and who is there", "В каком голосовом канале вы и кто там"),
    (
        "Who is in a voice channel, or in all voice channels of a server",
        "Кто в голосовом канале или во всех голосовых каналах сервера",
    ),
    (
        "Voice channel: Server/channel or an id; @name for a call in a chat",
        "Голосовой канал: Server/channel или id; @имя для звонка в чате",
    ),
    (
        "Show every voice channel of this server that has people in it",
        "Показать все голосовые каналы этого сервера, где есть люди",
    ),
    (
        "Join a voice channel (a call in a chat only if one is already running)",
        "Войти в голосовой канал (в звонок чата только если он уже идёт)",
    ),
    (
        "Voice channel: Server/channel or an id; @name for a running call in a chat",
        "Голосовой канал: Server/channel или id; @имя для идущего звонка в чате",
    ),
    (
        "Allow starting a NEW call in a chat: this rings the other people",
        "Разрешить начать НОВЫЙ звонок в чате: это позвонит другим людям",
    ),
    ("Leave the voice channel", "Выйти из голосового канала"),
    ("Mute your microphone", "Выключить микрофон"),
    ("Unmute your microphone", "Включить микрофон"),
    ("Stop hearing others", "Перестать слышать других"),
    ("Hear others again", "Снова слышать других"),
    ("List windows and screens that can be shared", "Список окон и экранов, которые можно показать"),
    ("Are you sharing, and where", "Идёт ли трансляция и куда"),
    (
        "Share a window or screen into the voice channel you are in (join one first)",
        "Показать окно или экран в голосовом канале, где вы сидите (сначала войдите в канал)",
    ),
    (
        "Window title (part of it) or an id from `stream list`",
        "Название окна (или его часть) или id из `stream list`",
    ),
    ("Share sound too (off by default)", "Передавать и звук (по умолчанию выключено)"),
    ("Stop sharing", "Остановить трансляцию"),
    (
        "Create an app with a token and the rights you approve (from flags or a manifest)",
        "Создать приложение с токеном и теми правами, что вы одобрите (из флагов или манифеста)",
    ),
    ("Apps and their rights", "Приложения и их права"),
    ("Delete an app: its token stops working at once", "Удалить приложение: его токен перестаёт работать сразу"),
    ("Last entries of the audit log", "Последние записи журнала"),
    (
        "Answer what apps ask to do (keep this running in its own window)",
        "Отвечать на запросы приложений (держите это открытым в отдельном окне)",
    ),
    (
        "Who the daemon thinks you are: the owner, or an app (depends on DISCORDCTL_TOKEN)",
        "Кем вас считает демон: владельцем или приложением (зависит от DISCORDCTL_TOKEN)",
    ),
    (
        "App name (letters, digits, - and _); optional when --manifest names it",
        "Имя приложения (буквы, цифры, - и _); не нужно, если его задаёт --manifest",
    ),
    (
        "Read the rights from a TOML manifest (see examples/agent.toml)",
        "Взять права из манифеста TOML (см. examples/agent.toml)",
    ),
    (
        "Lists it may see: servers, channels, chats, friends, me, members, voices, presence, or *",
        "Какие списки ему видны: servers, channels, chats, friends, me, members, voices, presence или *",
    ),
    (
        "Chats it may read: @name, Server/channel, Server/*, @*, or *",
        "Какие чаты можно читать: @имя, Server/channel, Server/*, @* или *",
    ),
    ("Chats it may write to (same forms)", "В какие чаты можно писать (те же формы)"),
    (
        "Voice actions: join, leave, mute, deafen, status, or *",
        "Голосовые действия: join, leave, mute, deafen, status или *",
    ),
    (
        "Voice channels it may join (same forms as --read)",
        "В какие голосовые каналы можно входить (те же формы, что у --read)",
    ),
    ("Sharing: list, status, stop, window, screen, or *", "Трансляция: list, status, stop, window, screen или *"),
    (
        "Do not ask before each call (dangerous: by default writes, voice and sharing wait for you)",
        "Не спрашивать перед каждым вызовом (опасно: по умолчанию запись, голос и трансляция ждут вас)",
    ),
    (
        "Create an app for the agent and register the MCP server in its config",
        "Создать приложение для агента и прописать MCP-сервер в его настройках",
    ),
    (
        "Which agent to connect (repeatable); asks if omitted",
        "Какого агента подключить (можно повторять); без значения спросит",
    ),
    ("What the agent may do", "Что разрешено агенту"),
    ("Skip the questions", "Не задавать вопросов"),
    ("Which agents are connected", "Какие агенты подключены"),
    (
        "Disconnect an agent: remove it from the config and revoke its token",
        "Отключить агента: убрать из настроек и отозвать токен",
    ),
    ("Start the daemon", "Запустить демон"),
    (
        "Stop the daemon (this closes a running Discord, so --force is required)",
        "Остановить демон (это закроет работающий Discord, поэтому нужен --force)",
    ),
    (
        "Call a payload method inside Discord: `dev call selfcheck`",
        "Вызвать метод payload внутри Discord: `dev call selfcheck`",
    ),
    ("Parameters as a JSON object", "Параметры в виде JSON-объекта"),
    (
        "Reload the Discord page with a payload bundle from disk",
        "Перезагрузить страницу Discord с payload-пакетом с диска",
    ),
];
