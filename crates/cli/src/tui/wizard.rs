//! Мастер установки на весь экран: проверка, параметры, установка, подключение Discord, агенты, итог.

use super::{bold, dim, fg, first_upper, height_of, panel, text, Choice, Flow, Item, Tui, ACCENT, BAD, DIM, OK, WARN};
use crate::{
    api::{ask, owner_call, stop_daemon},
    cli::{AgentKind, Preset, SetupArgs},
    doctor::{self, Level},
    mcp, ops, winsys,
};
use anyhow::{anyhow, bail, Result};
use crossterm::event::{KeyCode, KeyEvent};
use discordctl_core::{config::Config, paths, permissions::Permissions, protocol::Request};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::{
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

/// Итог для строки под экраном после выхода из полноэкранного режима.
pub struct Summary {
    pub attached: bool,
    pub agents: Vec<String>,
}

struct Facts {
    app_dir: Option<PathBuf>,
    installed: bool,
    running: bool,
    managed: bool,
    kaspersky: bool,
    others: Vec<&'static str>,
    on_path: bool,
}

impl Facts {
    fn gather(root: &Path) -> Facts {
        let managed =
            ask(&Request::Status).ok().flatten().and_then(|r| r.data).is_some_and(|d| !d["managed_discord"].is_null());
        Facts {
            app_dir: paths::latest_app_dir(root),
            installed: Config::load().shim_installed,
            running: ops::discord_running(root),
            managed,
            kaspersky: winsys::process_running("avp.exe"),
            others: ops::other_discord_builds(),
            on_path: !ops::path_managed() || ops::bin_on_path(),
        }
    }
}

struct Opts {
    path: bool,
    links: bool,
    restart: bool,
}

pub fn run(a: &SetupArgs) -> Result<Summary> {
    let root = ops::locate_discord(a.root.clone());
    let mut tui = Tui::new()?;
    let result = flow(&mut tui, a, root);
    drop(tui);
    result
}

fn flow(tui: &mut Tui, a: &SetupArgs, root: PathBuf) -> Result<Summary> {
    let facts = Facts::gather(&root);
    if !welcome(tui, &facts, &root)? {
        if facts.app_dir.is_none() {
            bail!(
                "Discord not found in {}: install the desktop app from discord.com first, or pass --root <folder>",
                root.display()
            );
        }
        bail!("cancelled");
    }
    let opts = options(tui, &facts, &root, a)?;
    install(tui, &facts, &opts, &root)?;
    let attached = connect(tui, &facts, &opts, &root)?;
    let (agents, snippets) = agents(tui, a)?;
    done(tui, attached, &agents, &snippets)?;
    Ok(Summary {
        attached,
        agents: agents.into_iter().filter_map(|(k, r)| r.is_ok().then(|| k.title().to_owned())).collect(),
    })
}

// ---- общие экраны --------------------------------------------------------------------------------------------

fn buttons(labels: &[&str], focus: usize) -> Line<'static> {
    let mut spans = vec![];
    for (i, l) in labels.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        let style = if i == focus {
            Style::new().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::Rgb(200, 205, 225)).bg(Color::Rgb(50, 54, 80))
        };
        spans.push(Span::styled(format!("  {l}  "), style));
    }
    Line::from(spans)
}

/// Движение по кнопкам. `true`, если нажат Enter.
fn button_key(focus: &mut usize, n: usize, k: KeyEvent) -> bool {
    match k.code {
        KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => *focus = (*focus + n - 1) % n,
        KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => *focus = (*focus + 1) % n,
        KeyCode::Enter => return true,
        _ => {}
    }
    false
}

/// Текст и ряд кнопок. Возвращает номер нажатой кнопки; Esc даёт `esc`.
fn ask_buttons(
    tui: &mut Tui,
    step: usize,
    title: &str,
    lines: Vec<Line<'static>>,
    labels: &[&str],
    default: usize,
    esc: usize,
) -> Result<usize> {
    let mut focus = default;
    let n = labels.len();
    tui.run(
        &mut focus,
        step,
        title,
        &[("←→", "choose"), ("Enter", "confirm")],
        |f, area, focus, _| {
            let [body, _, row] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1), Constraint::Length(1)]).areas(area);
            text(f, body, lines.clone());
            f.render_widget(Paragraph::new(buttons(labels, *focus)), row);
        },
        |focus, k| {
            if k.code == KeyCode::Esc {
                return Flow::Done(esc);
            }
            if button_key(focus, n, k) {
                Flow::Done(*focus)
            } else {
                Flow::Stay
            }
        },
    )
}

fn plain(s: impl Into<String>) -> Line<'static> {
    Line::from(s.into())
}

fn bullet(s: impl Into<String>) -> Line<'static> {
    Line::from(vec![Span::styled("  •  ", fg(ACCENT)), Span::raw(s.into())])
}

fn error_screen(tui: &mut Tui, step: usize, title: &str, e: &anyhow::Error, hint: &str) -> Result<()> {
    let mut lines = vec![Line::from(Span::styled(format!("✖ {e}"), bold(BAD)))];
    for cause in e.chain().skip(1) {
        lines.push(Line::from(Span::styled(format!("   caused by: {cause}"), dim())));
    }
    lines.push(plain(""));
    lines.push(Line::from(Span::styled(hint.to_owned(), dim())));
    ask_buttons(tui, step, title, lines, &["Close"], 0, 0)?;
    Ok(())
}

// ---- 1. проверка ---------------------------------------------------------------------------------------------

struct Row {
    mark: &'static str,
    color: Color,
    label: &'static str,
    detail: String,
}

fn check_rows(facts: &Facts, root: &Path) -> Vec<Row> {
    let mut rows = vec![];
    match &facts.app_dir {
        Some(d) => rows.push(Row {
            mark: "✔",
            color: OK,
            label: "Discord",
            detail: format!(
                "{} · {}",
                d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                root.display()
            ),
        }),
        None => rows.push(Row {
            mark: "✖",
            color: BAD,
            label: "Discord",
            detail: if facts.others.is_empty() {
                format!("not found in {}", root.display())
            } else {
                format!("only {} found; just the stable build is supported", facts.others.join(", "))
            },
        }),
    }
    rows.push(Row {
        mark: if facts.installed { "✔" } else { "•" },
        color: if facts.installed { OK } else { DIM },
        label: "discordctl",
        detail: if facts.installed {
            "already installed, files will be refreshed".into()
        } else {
            "not installed yet".into()
        },
    });
    if facts.app_dir.is_some() {
        rows.push(Row {
            mark: if facts.running { "!" } else { "•" },
            color: if facts.running { WARN } else { DIM },
            label: "Discord state",
            detail: match (facts.running, facts.managed) {
                (true, true) => "running under discordctl; it must restart to update the daemon".into(),
                (true, false) => "running without discordctl; it has to restart to attach".into(),
                _ => "not running".into(),
            },
        });
    }
    if facts.kaspersky {
        rows.push(Row {
            mark: "•",
            color: DIM,
            label: "Antivirus",
            detail: "Kaspersky is running; if it blocks a file, `discordctl doctor` explains what to do".into(),
        });
    }
    rows
}

fn rows_to_lines(rows: &[Row]) -> Vec<Line<'static>> {
    rows.iter()
        .map(|r| {
            Line::from(vec![
                Span::styled(format!("{} ", r.mark), bold(r.color)),
                Span::styled(format!("{:<15}", r.label), bold(Color::White)),
                Span::raw(r.detail.clone()),
            ])
        })
        .collect()
}

const TOS: &str = "discordctl drives your own user account through the desktop client. Discord's Terms of Service forbid that and an account can be banned. Reads and writes are rate-limited and nothing pings anyone by default, but there is no guarantee. Use it at your own risk; this project is not affiliated with Discord.";

fn welcome_body(f: &mut Frame, area: Rect, facts: &Facts, root: &Path, focus: usize, labels: &[&str]) {
    let rows = rows_to_lines(&check_rows(facts, root));
    let tos = vec![Line::from(Span::raw(TOS))];
    let rows_h = height_of(&rows, area.width.saturating_sub(4)) + 2;
    let tos_h = height_of(&tos, area.width.saturating_sub(4)) + 2;
    let [a, b, _, c] = Layout::vertical([
        Constraint::Length(rows_h),
        Constraint::Length(tos_h),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let inner = panel(f, a, "Your system", ACCENT);
    text(f, inner, rows);
    let inner = panel(f, b, "Read this first", WARN);
    text(f, inner, tos);
    f.render_widget(Paragraph::new(buttons(labels, focus)), c);
}

fn welcome(tui: &mut Tui, facts: &Facts, root: &Path) -> Result<bool> {
    let labels: Vec<&str> = if facts.app_dir.is_some() { vec!["Continue", "Quit"] } else { vec!["Quit"] };
    let n = labels.len();
    let mut focus = 0usize;
    let pick = tui.run(
        &mut focus,
        0,
        "Welcome — let's check your system",
        &[("←→", "choose"), ("Enter", "confirm"), ("Esc", "quit")],
        |f, area, focus, _| welcome_body(f, area, facts, root, *focus, &labels),
        |focus, k| {
            if k.code == KeyCode::Esc {
                return Flow::Done(n - 1);
            }
            if button_key(focus, n, k) {
                Flow::Done(*focus)
            } else {
                Flow::Stay
            }
        },
    )?;
    Ok(facts.app_dir.is_some() && pick == 0)
}

// ---- 2. параметры --------------------------------------------------------------------------------------------

fn options(tui: &mut Tui, facts: &Facts, root: &Path, a: &SetupArgs) -> Result<Opts> {
    let mut items = vec![];
    if ops::path_managed() && !facts.on_path {
        items.push(Item::toggle(
            "Add discordctl to PATH",
            "so `discordctl` works from any terminal window (open a new one afterwards)",
            true,
        ));
    }
    let mut links = Item::toggle(
        "Open discord:// links with discordctl",
        "changes one registry value (HKCU); optional, some antivirus programs dislike it",
        Config::load().handler_hook,
    );
    links.tag = None;
    items.push(links);
    let path_index = (ops::path_managed() && !facts.on_path).then_some(0);
    let links_index = items.len() - 1;
    let mut restart_index = None;
    if facts.running {
        let mut it = Item::toggle(
            "Restart Discord now",
            "needed so discordctl can attach to it; a running call will be interrupted",
            !a.no_restart,
        );
        if facts.managed {
            it.on = true;
            it = it.locked();
        }
        restart_index = Some(items.len());
        items.push(it);
    }
    items.push(Item::action("Install", ""));
    let mut choice = Choice::new(items);

    let summary = vec![
        bullet(format!("copy its programs to {}", paths::bin_dir().display())),
        bullet("put a small launcher in front of Update.exe and Discord.exe (originals stay as *.real.exe)"),
        bullet("everything is reverted by `discordctl uninstall`; the launcher falls back to the real Discord on any error"),
    ];
    let discord_line = format!("Discord at {}", root.display());
    tui.run(
        &mut choice,
        1,
        "Choose what to install",
        &[("↑↓", "move"), ("Space", "toggle"), ("Enter", "select / continue"), ("Esc", "quit")],
        |f, area, choice, _| {
            let sum_h = summary.len() as u16 + 3;
            let [top, list] = Layout::vertical([Constraint::Length(sum_h), Constraint::Min(1)]).areas(area);
            let mut lines = vec![Line::from(Span::styled(discord_line.clone(), dim()))];
            lines.push(plain("discordctl will:"));
            lines.extend(summary.clone());
            text(f, top, lines);
            text(f, list, choice.lines());
        },
        |choice, k| {
            if k.code == KeyCode::Esc {
                return Flow::Done(None);
            }
            if choice.key(k) {
                return Flow::Stay;
            }
            if k.code == KeyCode::Enter {
                if choice.on_action() {
                    return Flow::Done(Some(()));
                }
                choice.toggle();
            }
            Flow::Stay
        },
    )?
    .ok_or_else(|| anyhow!("cancelled"))?;

    let on = |i: Option<usize>| i.is_some_and(|i| choice.items[i].on);
    Ok(Opts { path: on(path_index), links: choice.items[links_index].on, restart: on(restart_index) })
}

// ---- 3. установка --------------------------------------------------------------------------------------------

fn install(tui: &mut Tui, facts: &Facts, opts: &Opts, root: &Path) -> Result<()> {
    let result = tui.work(2, "Installing", |p: &dyn Fn(&str)| -> Result<()> {
        if facts.running && opts.restart {
            p("Closing Discord");
            let _ = stop_daemon(true); // без демона команда ничего не делает
            if !ops::close_discord(root) {
                bail!("could not close Discord: quit it from the tray icon and run setup again");
            }
        }
        ops::install_with(root.to_path_buf(), Some(opts.links), &mut |s: &str| p(&first_upper(s)))?;
        if opts.path && ops::path_managed() && !ops::bin_on_path() {
            p("Adding discordctl to PATH");
            ops::add_bin_to_path()?;
        }
        p("Done");
        Ok(())
    })?;
    if let Err(e) = result {
        error_screen(tui, 2, "Installation failed", &e, "Nothing was left half-done on purpose: `discordctl doctor` shows the state, `discordctl uninstall` reverts it.")?;
        return Err(e);
    }
    Ok(())
}

// ---- 4. подключение к Discord --------------------------------------------------------------------------------

/// Ждёт, пока хук в Discord станет активным.
fn wait_hook(p: &dyn Fn(&str)) -> bool {
    // Для тестов срок можно сократить: поддельному Discord соединяться нечем.
    let secs: u64 = std::env::var("DISCORDCTL_SETUP_WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(90);
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Ok(Some(r)) = ask(&Request::Status) {
            let hook = r.data.map(|d| d["hook"].clone()).unwrap_or_default();
            match hook["phase"].as_str().unwrap_or("") {
                "active" => return true,
                ph @ ("waiting_page" | "installing" | "waiting_payload") => p(&format!("~Discord is loading ({ph})")),
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(700));
    }
    false
}

fn launch_and_wait(p: &dyn Fn(&str)) -> bool {
    p("Starting Discord under discordctl");
    match owner_call(&Request::Launch { args: vec![] }) {
        Ok(_) => wait_hook(p),
        Err(e) => {
            p(&format!("~{e:#}"));
            false
        }
    }
}

fn connect(tui: &mut Tui, facts: &Facts, opts: &Opts, root: &Path) -> Result<bool> {
    let restarted = facts.running && opts.restart;
    let mut attached = false;
    if restarted || !facts.running {
        attached = tui.work(3, "Starting Discord", launch_and_wait)?;
    }
    for round in 0..2 {
        if attached {
            return Ok(true);
        }
        let first = round == 0;
        let lines = vec![
            Line::from(Span::styled(
                if restarted || !facts.running {
                    "Discord started, but discordctl could not attach to it yet."
                } else {
                    "Discord is running without discordctl."
                },
                bold(WARN),
            )),
            plain(""),
            plain("A freshly installed Discord updates itself and restarts, which drops the connection."),
            plain("Restarting it once under discordctl normally fixes that. A running call will be interrupted."),
        ];
        let labels: &[&str] =
            if first { &["Restart Discord under discordctl", "Continue without"] } else { &["Continue without"] };
        let pick = ask_buttons(tui, 3, "Connect to Discord", lines, labels, 0, labels.len() - 1)?;
        if !first || pick == 1 {
            return Ok(false);
        }
        attached = tui.work(3, "Restarting Discord", |p: &dyn Fn(&str)| {
            p("Closing Discord");
            let _ = stop_daemon(true);
            if !ops::close_discord(root) {
                p("~could not close Discord");
                return false;
            }
            launch_and_wait(p)
        })?;
    }
    Ok(attached)
}

// ---- 5. агенты ----------------------------------------------------------------------------------------------

type Connected = Vec<(AgentKind, Result<String>)>;

fn agent_desc(a: AgentKind) -> &'static str {
    match a {
        AgentKind::ClaudeCode => "adds an MCP server with `claude mcp add` (needs the `claude` command)",
        AgentKind::ClaudeDesktop => "adds the server to claude_desktop_config.json",
        AgentKind::Cursor => "adds the server to ~/.cursor/mcp.json",
        AgentKind::Print => "touch nothing: show the settings so you can paste them yourself",
    }
}

fn pick_agents(tui: &mut Tui) -> Result<Vec<AgentKind>> {
    let avail = mcp::available();
    let none_found = avail == [AgentKind::Print];
    let mut items: Vec<Item> = avail
        .iter()
        .map(|a| {
            let found = *a != AgentKind::Print;
            let mut it = Item::toggle(a.title(), agent_desc(*a), found);
            if found {
                it = it.tag("found on this PC", OK);
            }
            it
        })
        .collect();
    items.push(Item::action("Connect", ""));
    items.push(Item::action("Skip", ""));
    let mut choice = Choice::new(items);
    let n = avail.len();
    let picked = tui.run(
        &mut choice,
        4,
        "Connect AI agents (optional)",
        &[("↑↓", "move"), ("Space", "toggle"), ("Enter", "select"), ("Esc", "skip")],
        |f, area, choice, _| {
            let [top, list] = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(area);
            text(
                f,
                top,
                vec![
                    plain(if none_found {
                        "No Claude Code, Claude Desktop or Cursor was found on this PC."
                    } else {
                        "Give an agent access to your Discord over MCP."
                    }),
                    Line::from(Span::styled(
                        if none_found {
                            "You can still get the settings to paste into any agent that supports MCP."
                        } else {
                            "It gets a token with only the rights you approve on the next screen."
                        },
                        dim(),
                    )),
                ],
            );
            text(f, list, choice.lines());
        },
        |choice, k| {
            if k.code == KeyCode::Esc {
                return Flow::Done(None);
            }
            if choice.key(k) {
                return Flow::Stay;
            }
            if k.code == KeyCode::Enter {
                if !choice.on_action() {
                    choice.toggle();
                } else if choice.cursor == n + 1 {
                    return Flow::Done(None);
                } else {
                    return Flow::Done(Some(
                        (0..n).filter(|i| choice.items[*i].on).map(|i| avail[i]).collect::<Vec<_>>(),
                    ));
                }
            }
            Flow::Stay
        },
    )?;
    Ok(picked.unwrap_or_default())
}

fn pick_preset(tui: &mut Tui) -> Result<Preset> {
    let mut choice = Choice::new(vec![
        Item::radio("Read-only", "list chats and read messages; nothing can be sent, said or shared", true),
        Item::radio(
            "Assistant",
            "read, plus send / edit / delete / react; every write still waits for your approval",
            false,
        ),
        Item::action("Continue", ""),
    ]);
    tui.run(
        &mut choice,
        4,
        "What may the agent do?",
        &[("↑↓", "move"), ("Space", "select"), ("Enter", "continue")],
        |f, area, choice, _| text(f, area, choice.lines()),
        |choice, k| {
            if choice.key(k) {
                return Flow::Stay;
            }
            if k.code == KeyCode::Enter {
                if choice.on_action() {
                    return Flow::Done(if choice.items[1].on { Preset::Assistant } else { Preset::ReadOnly });
                }
                choice.toggle();
            }
            Flow::Stay
        },
    )
}

fn rights_lines(p: &Permissions) -> Vec<Line<'static>> {
    let row = |label: &'static str, items: &[String], risky: bool| -> Option<Line<'static>> {
        (!items.is_empty()).then(|| {
            Line::from(vec![
                Span::styled(format!("{label:<8}"), bold(if risky { WARN } else { ACCENT })),
                Span::raw(items.join(", ")),
            ])
        })
    };
    [
        row("list", &p.list, false),
        row("read", &p.read, false),
        row("write", &p.write, true),
        row("voice", &p.voice, true),
        row("stream", &p.stream, true),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn review(tui: &mut Tui, agents: &[AgentKind], preset: Preset, perms: &Permissions) -> Result<bool> {
    let names = agents.iter().map(|a| a.title()).collect::<Vec<_>>().join(", ");
    let writes = preset == Preset::Assistant;
    let mut lines = vec![
        Line::from(vec![
            Span::styled(names, bold(Color::White)),
            Span::raw(" will get access to your Discord account:"),
        ]),
        plain(""),
    ];
    lines.extend(rights_lines(perms));
    lines.push(plain(""));
    lines.push(Line::from(Span::styled("`*` means every chat and server you can see now and in the future.", dim())));
    lines.push(Line::from(Span::styled(
        if writes {
            "Every write waits for your OK: keep `discordctl app approve` open in another window."
        } else {
            "Read-only: nothing can be sent, said or shared.  Revoke any time: `discordctl mcp remove`."
        },
        if writes { fg(WARN) } else { dim() },
    )));
    let pick =
        ask_buttons(tui, 4, "Approve these rights", lines, &["Allow", "Don't allow"], if writes { 1 } else { 0 }, 1)?;
    Ok(pick == 0)
}

fn agents(tui: &mut Tui, a: &SetupArgs) -> Result<(Connected, Vec<String>)> {
    let (picked, preset) = if a.agent.is_empty() {
        let picked = pick_agents(tui)?;
        if picked.is_empty() {
            return Ok((vec![], vec![]));
        }
        (picked, pick_preset(tui)?)
    } else {
        (a.agent.clone(), a.preset)
    };
    let perms = mcp::preset_permissions(preset);
    if !review(tui, &picked, preset, &perms)? {
        return Ok((vec![], vec![]));
    }
    let results = tui.work(4, "Connecting agents", |p: &dyn Fn(&str)| {
        picked
            .iter()
            .map(|k| {
                p(&format!("Connecting {}", k.title()));
                (*k, mcp::connect_quiet(*k, &perms))
            })
            .collect::<Vec<_>>()
    })?;
    let mut snippets = vec![];
    let connected = results
        .into_iter()
        .map(|(k, r)| match r {
            Ok((how, snippet)) => {
                snippets.extend(snippet);
                (k, Ok(how))
            }
            Err(e) => (k, Err(e)),
        })
        .collect();
    Ok((connected, snippets))
}

// ---- 6. итог -------------------------------------------------------------------------------------------------

fn done_lines(
    checks: &[doctor::Check],
    fixed_attached: bool,
    agents: &Connected,
    snippets: &[String],
) -> (Vec<Line<'static>>, bool) {
    let bad = checks.iter().any(|c| c.level == Level::Fail);
    let warn = checks.iter().any(|c| c.level == Level::Warn);
    let mut out = vec![];
    out.push(Line::from(Span::styled(
        match (bad, warn || !fixed_attached) {
            (true, _) => "Installed, but something needs attention",
            (false, true) => "Installed, with a note",
            _ => "Everything is in place",
        },
        bold(if bad {
            BAD
        } else if warn || !fixed_attached {
            WARN
        } else {
            OK
        }),
    )));
    out.push(plain(""));
    for c in checks {
        let (mark, color) = match c.level {
            Level::Ok => ("✔", OK),
            Level::Info => ("•", DIM),
            Level::Warn => ("!", WARN),
            Level::Fail => ("✖", BAD),
        };
        out.push(Line::from(vec![
            Span::styled(format!("{mark} "), bold(color)),
            Span::styled(format!("{:<22}", c.name), bold(Color::White)),
            Span::raw(c.detail.clone()),
        ]));
        if let (true, Some(h)) = (c.level >= Level::Warn, &c.hint) {
            out.push(Line::from(Span::styled(format!("    → {h}"), dim())));
        }
    }
    if !agents.is_empty() {
        out.push(plain(""));
        out.push(Line::from(Span::styled("AI agents", bold(ACCENT))));
        for (k, r) in agents {
            match r {
                Ok(how) => out.push(Line::from(vec![
                    Span::styled("✔ ", bold(OK)),
                    Span::styled(format!("{:<16}", k.title()), bold(Color::White)),
                    Span::raw(how.clone()),
                ])),
                Err(e) => out.push(Line::from(vec![
                    Span::styled("✖ ", bold(BAD)),
                    Span::styled(format!("{:<16}", k.title()), bold(Color::White)),
                    Span::raw(format!("{e:#}")),
                ])),
            }
        }
        if agents.iter().any(|(_, r)| r.is_ok()) {
            out.push(Line::from(Span::styled("Restart the agent so it picks up the new server.", dim())));
        }
    }
    for s in snippets {
        out.push(plain(""));
        out.extend(s.lines().map(|l| Line::from(Span::styled(l.to_owned(), fg(Color::Rgb(190, 195, 215))))));
    }
    out.push(plain(""));
    out.push(Line::from(Span::styled("Try", bold(ACCENT))));
    for (cmd, what) in [
        ("discordctl chat", "your direct chats and groups"),
        ("discordctl read @name -n 20", "the last messages of a chat"),
        ("discordctl doctor", "check the installation any time"),
    ] {
        out.push(Line::from(vec![Span::styled(format!("  {cmd:<30}"), bold(Color::White)), Span::styled(what, dim())]));
    }
    (out, bad)
}

fn done(tui: &mut Tui, attached: bool, agents: &Connected, snippets: &[String]) -> Result<()> {
    let checks = tui.work(5, "Checking the result", |p: &dyn Fn(&str)| {
        p("Running the health check");
        doctor::collect(false).map(|(c, _)| c).unwrap_or_default()
    })?;
    let (lines, _) = done_lines(&checks, attached, agents, snippets);
    let mut scroll = 0u16;
    let total = lines.len() as u16;
    tui.run(
        &mut scroll,
        5,
        "Done",
        &[("↑↓", "scroll"), ("Enter", "finish")],
        |f, area, scroll, _| {
            let max = total.saturating_sub(area.height);
            f.render_widget(Paragraph::new(lines.clone()).scroll(((*scroll).min(max), 0)), area);
        },
        |scroll, k| {
            match k.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => return Flow::Done(()),
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::PageDown => *scroll = scroll.saturating_add(8),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(8),
                _ => {}
            }
            Flow::Stay
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn dump(t: &Terminal<TestBackend>) -> String {
        let buf = t.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_owned()).collect::<String>().trim_end().to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn facts(found: bool) -> Facts {
        Facts {
            app_dir: found.then(|| PathBuf::from(r"C:\Users\x\AppData\Local\Discord\app-1.0.9265")),
            installed: false,
            running: true,
            managed: false,
            kaspersky: true,
            others: vec![],
            on_path: false,
        }
    }

    #[test]
    fn welcome_screen_renders() {
        let mut t = Terminal::new(TestBackend::new(100, 28)).unwrap();
        let root = PathBuf::from(r"C:\Users\x\AppData\Local\Discord");
        t.draw(|f| {
            let area = super::super::shell(f, 0, "Welcome — let's check your system", &[("Enter", "confirm")]);
            welcome_body(f, area, &facts(true), &root, 0, &["Continue", "Quit"]);
        })
        .unwrap();
        let s = dump(&t);
        println!("{s}");
        assert!(s.contains("discordctl") && s.contains("Your system") && s.contains("Read this first"));
        assert!(s.contains("Continue") && s.contains("Quit"));
    }

    #[test]
    fn missing_discord_offers_only_quit() {
        let mut t = Terminal::new(TestBackend::new(100, 28)).unwrap();
        let root = PathBuf::from(r"C:\nope");
        t.draw(|f| {
            let area = super::super::shell(f, 0, "Welcome", &[]);
            welcome_body(f, area, &facts(false), &root, 0, &["Quit"]);
        })
        .unwrap();
        let s = dump(&t);
        assert!(s.contains("not found in C:\\nope"));
    }

    #[test]
    fn choice_list_renders_toggles_and_actions() {
        let mut t = Terminal::new(TestBackend::new(100, 20)).unwrap();
        let mut c = Choice::new(vec![
            Item::toggle("Add discordctl to PATH", "so it works anywhere", true),
            Item::toggle("Restart Discord now", "needed to attach", true).locked(),
            Item::radio("Read-only", "", false),
            Item::action("Install", ""),
        ]);
        c.down();
        t.draw(|f| {
            let area = super::super::shell(f, 1, "Choose what to install", &[("Space", "toggle")]);
            text(f, area, c.lines());
        })
        .unwrap();
        let s = dump(&t);
        println!("{s}");
        assert!(s.contains("[✔] Add discordctl to PATH"));
        assert!(s.contains("❯ [✔] Restart Discord now") && s.contains("(required)"));
        assert!(s.contains("( ) Read-only") && s.contains(" Install "));
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn dump(t: &Terminal<TestBackend>) -> String {
        let buf = t.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_owned()).collect::<String>().trim_end().to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn done_screen_lists_checks_agents_and_next_steps() {
        let checks = vec![
            doctor::Check { name: "Discord install", level: Level::Ok, detail: "app-1.0.9265".into(), hint: None },
            doctor::Check {
                name: "hook",
                level: Level::Warn,
                detail: "waiting_page".into(),
                hint: Some("Wait for Discord to finish loading.".into()),
            },
        ];
        let agents: Connected = vec![
            (AgentKind::ClaudeCode, Ok("registered with `claude mcp add` (user scope)".into())),
            (AgentKind::Cursor, Err(anyhow!("cannot write the config"))),
        ];
        let (lines, bad) = done_lines(&checks, false, &agents, &[]);
        assert!(!bad);
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| {
            let area = super::super::shell(f, 5, "Done", &[("Enter", "finish")]);
            f.render_widget(Paragraph::new(lines.clone()), area);
        })
        .unwrap();
        let s = dump(&t);
        println!("{s}");
        assert!(s.contains("Installed, with a note") && s.contains("Claude Code") && s.contains("discordctl chat"));
        assert!(s.contains("✖ Cursor") && s.contains("cannot write the config"));
    }

    #[test]
    fn rights_review_screen_shows_what_is_granted() {
        let perms = mcp::preset_permissions(Preset::Assistant);
        let mut lines = vec![Line::from("Claude Code will get access:")];
        lines.extend(rights_lines(&perms));
        let mut t = Terminal::new(TestBackend::new(100, 20)).unwrap();
        t.draw(|f| {
            let area = super::super::shell(f, 4, "Approve these rights", &[("←→", "choose")]);
            f.render_widget(Paragraph::new(lines.clone()), area);
        })
        .unwrap();
        let s = dump(&t);
        println!("{s}");
        assert!(s.contains("list") && s.contains("read") && s.contains("write"));
    }

    #[test]
    fn narrow_terminal_does_not_panic() {
        for (w, h) in [(40, 12), (20, 6), (100, 5), (3, 3)] {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            let c = Choice::new(vec![
                Item::toggle("A long title for a narrow window", "and a longer description text", true),
                Item::action("Install", ""),
            ]);
            t.draw(|f| {
                let area = super::super::shell(f, 1, "Title", &[("Enter", "ok")]);
                text(f, area, c.lines());
            })
            .unwrap();
        }
    }
}
