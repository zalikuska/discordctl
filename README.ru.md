# discordctl

**Ваш Discord из терминала и из ИИ-агентов.**
Чтение и отправка сообщений, голосовые каналы, трансляция окна и подключение Claude Code, Claude Desktop или Cursor
через MCP с правами, которые вы одобряете построчно.

[English](README.md) · [Архитектура](docs/ARCHITECTURE.md) (на английском) · [Безопасность](SECURITY.md)

> [!WARNING]
> discordctl автоматизирует **ваш собственный пользовательский аккаунт** через десктопный клиент. Это нарушает условия
> использования Discord и может привести к блокировке аккаунта. Чтение и запись ограничены по частоте, записи по
> умолчанию никого не пингуют, но гарантий нет. Вы используете это на свой риск. Проект не связан с Discord.

Нужны Windows 10/11 и официальное десктопное приложение Discord.

## Установка

**Одной строкой (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/zalikuska/discordctl/main/scripts/install.ps1 | iex
```

Скрипт скачает последний релиз, проверит SHA-256 и запустит `discordctl setup`: мастер спрашивает перед каждым
изменением. Он сам найдёт Discord (обычная папка или запись об удалении в Windows), установит программы, запустит
Discord под управлением discordctl, проверит результат и предложит выбрать ИИ-агентов (Claude Code, Claude Desktop,
Cursor). Если десктопного Discord нет, скрипт предложит скачать официальный установщик с discord.com (сначала
проверит подпись Discord Inc.). Для установки без вопросов задайте `DISCORDCTL_ARGS`, например
`$env:DISCORDCTL_ARGS = '--yes --agent claude-code --preset read-only'` перед командой.

Хотите сначала посмотреть? Скачайте `discordctl-*-windows-x64.zip` из
[Releases](https://github.com/zalikuska/discordctl/releases), распакуйте и запустите `discordctl.exe setup`.

**Из исходников** (Rust 1.80+, Node 20+ нужен только при правках `payload/`):

```powershell
git clone https://github.com/zalikuska/discordctl; cd discordctl
cargo build --release
.\target\release\discordctl.exe setup
```

Мастер открывается на весь экран (стрелки, пробел, Enter; `--plain` включает простые вопросы) и делает пять шагов: проверяет Discord, ставит маленький лаунчер перед `Update.exe` и `Discord.exe` (оригиналы
сохраняются), запускает Discord под управлением discordctl, проверяет здоровье установки и предлагает подключить
ИИ-агента. Всё откатывается командой `discordctl uninstall`.

> Программы не подписаны и вмешиваются в запуск чужой программы, поэтому антивирус может ругаться. Установка и удаление не запускают
> `powershell` и меняют ссылки `discord://` только по вашему желанию (`--link-hook`). `discordctl doctor`
> подскажет, что делать.

## Быстрый старт

```powershell
discordctl chat                           # личные чаты и группы
discordctl read @alice -n 20              # последние 20 сообщений
discordctl send @alice "привет" --dry-run # предпросмотр; уберите --dry-run, чтобы отправить
discordctl voice join "Мой сервер/Общий"  # войти в голосовой канал
discordctl voice who --server "Мой сервер"  # кто в каких голосовых каналах
discordctl server members "Мой сервер" -f аня  # участники, которых клиент уже загрузил
discordctl stream list                    # окна, которые можно транслировать
discordctl doctor                         # всё ли в порядке?
discordctl update                         # скачать, проверить и поставить свежий релиз (--check: только посмотреть)
discordctl lang ru                        # русский интерфейс (en, ru, auto; ещё --lang или DISCORDCTL_LANG; Ctrl+L в мастере)
```

### Цель команды: один синтаксис везде

| Пишете | Значит |
|---|---|
| `@alice` или `"@Название группы"` | личный чат или группа по имени, нику или участнику |
| `"Мой сервер/общий"` | канал сервера |
| `123456789012345678` | любой id |
| `@*`, `"Мой сервер/*"`, `*` | (только для прав) все чаты, весь сервер, всё |

## Команды

```text
setup | install | update | uninstall | doctor | status | start  жизненный цикл
server | channel list <сервер> | chat | friend                  что у вас есть
message read|send|edit|delete|react <цель> …                    (короткие формы: read, send)
voice status|who|join|leave|mute|unmute|deafen|undeafen
stream list|status|start|stop
app add|list|remove|log|approve|whoami                          права приложений и агентов
mcp install|status|remove                                       подключение ИИ-агентов
daemon start|stop · completions <оболочка>
```

У каждой команды есть `--help`, а для скриптов `--json`. `--no-color` (или `NO_COLOR`) выключает цвета.

## ИИ-агенты (MCP)

```powershell
discordctl mcp install     # выберите агентов и набор прав; права нужно одобрить
discordctl app approve     # держите открытым в другом окне: записи ждут вашего «да» здесь
```

Наборы: **read-only** (списки и чтение) и **assistant** (плюс отправка, правка, удаление, реакции, каждое действие вы
одобряете) и **autonomous** (всё, включая голос и показ экрана, вообще без одобрений: только для агента, которому вы
полностью доверяете; `discordctl mcp install --preset autonomous` или пункт в мастере). Голос и трансляцию окна без
этого набора выдают явно через `discordctl app add`.

- Агент получает **токен только с одобренными правами**; отзыв мгновенный: `discordctl mcp remove` или
  `discordctl app remove <имя>`.
- MCP-сервер не запустится без токена, показывает агенту только разрешённые инструменты и помечает всё, что пришло из
  Discord, как недоверенные данные, чтобы чужие сообщения хуже управляли моделью.
- Все решения пишутся в журнал: `discordctl app log`.

> [!IMPORTANT]
> Токен ограничивает *MCP-подключение*, но не агента, у которого есть обычная оболочка от вашего имени: такой агент может
> вызвать `discordctl` как владелец. Если граница важна, давайте агентам MCP, а не оболочку. См. [SECURITY.md](SECURITY.md).

## Если что-то не работает

Сначала `discordctl doctor`: он объясняет проблемы, а `discordctl doctor --fix` чинит то, что безопасно чинить.

- **Discord открыт, но не подключён.** Он запущен до установки или мимо лаунчера. Закройте его полностью (в том числе в
  трее) и откройте заново.
- **Самопроверка падает после обновления Discord.** Discord переименовал внутренние имена; нужно обновить payload
  ([как](docs/ARCHITECTURE.md#adapting-to-a-new-discord-build)).
- **После пересборки из исходников** снова выполните `discordctl install`: демон работает из `%LOCALAPPDATA%\discordctl\bin`.
- **Антивирус (например Kaspersky) заблокировал или убрал файл.** Восстановите его, добавьте в доверенные
  `%LOCALAPPDATA%\discordctl` и папку, из которой вы запускаете, затем `discordctl doctor --fix`.

## Удаление

```powershell
discordctl uninstall            # возвращает Discord; запущенный Discord не закрывает
discordctl uninstall --purge    # ещё удаляет токены, журнал и логи (спросит подтверждение)
```

Разработка и вклад: [CONTRIBUTING.md](CONTRIBUTING.md). Лицензия: [MIT](LICENSE). Комментарии в коде частично на русском.
