//! discordctl — управление своим клиентом Discord из терминала и из ИИ-агентов.

mod api;
mod approve;
mod cli;
mod doctor;
mod http;
mod i18n;
mod i18n_ru;
mod i18n_ru_help;
mod mcp;
mod ops;
mod render;
mod setup;
mod tui;
mod ui;
mod update;
mod winsys;
mod zipread;

use anyhow::{bail, Context, Result};
use api::{ask, out, owner_call, payload_call, T};
use clap::{CommandFactory, FromArgMatches};
use cli::*;
use discordctl_core::{
    client,
    config::Config,
    permissions::{Manifest, Permissions},
    protocol::Request,
    shim::{self, Target},
};
use serde_json::{json, Value};

fn main() {
    // Язык нужен до разбора аргументов: справка тоже печатается на выбранном языке.
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    i18n::set(i18n::detect(&args).resolve());
    let mut command = Cli::command();
    if i18n::current() == i18n::Lang::Ru {
        command = i18n::localize_command(command);
    }
    let matches = command.get_matches_from(&args);
    let parsed = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => e.exit(),
    };
    ui::init(parsed.no_color);
    if let Err(e) = run(parsed) {
        ui::print_error(&e);
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    match cli.cmd {
        Cmd::Setup(args) => {
            if json {
                let r = ops::install(ops::locate_discord(args.root), None)?;
                out(true, &r, || {});
                return Ok(());
            }
            setup::run(args)
        }
        Cmd::Install { root, link_hook } => {
            let root = ops::locate_discord(root);
            if json {
                let r = ops::install(root, link_hook.then_some(true))?;
                out(true, &r, || {});
                return Ok(());
            }
            let mut sp = ui::Spinner::start("installing");
            let mut step = |s: &str| sp.update(s.to_owned());
            match ops::install_with(root, link_hook.then_some(true), &mut step) {
                Ok(r) => {
                    sp.done(if r["changed"] == true { "installed" } else { "already installed, files refreshed" });
                    ui::info(format!("discord:// links: {}", r["link_handler"].as_str().unwrap_or("?")));
                    ui::hint(
                        "Start Discord as usual: the daemon launches it. Check everything with `discordctl doctor`.",
                    );
                    Ok(())
                }
                Err(e) => {
                    sp.failed("installation failed");
                    Err(e)
                }
            }
        }
        Cmd::Uninstall { purge } => {
            let r = ops::uninstall(purge)?;
            out(json, &r, || {
                if r["changed"] == true {
                    ui::ok("original Update.exe and Discord.exe restored");
                } else {
                    ui::info("nothing to restore (the shim was not installed)");
                }
                ui::info(format!("daemon: {}", i18n::tr(r["daemon"].as_str().unwrap_or("?"))));
                if r["purged"] == true {
                    ui::ok(format!("data deleted: {}", r["data_dir"].as_str().unwrap_or("")));
                } else {
                    ui::hint(format!(
                        "Your settings, app tokens and audit log stay in {} (use --purge to delete them).",
                        r["data_dir"].as_str().unwrap_or("")
                    ));
                }
            });
            Ok(())
        }
        Cmd::Update(args) => update::run(args, json),
        Cmd::Lang { value } => language(value, json),
        Cmd::Doctor { fix } => {
            if !doctor::run(json, fix)? {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Status => status(json),
        Cmd::Start { args } => {
            let r = client::call(&Request::Launch { args }, T).map_err(anyhow::Error::msg)?;
            if !r.ok {
                bail!("{}", r.msg.unwrap_or_default());
            }
            out(json, &serde_json::to_value(&r)?, || ui::ok(r.msg.clone().unwrap_or_default()));
            Ok(())
        }

        Cmd::Server { cmd: None | Some(ServerCmd::List) } => {
            let v = payload_call("guilds", json!({}))?;
            out(json, &v, || render::servers(&v));
            Ok(())
        }
        Cmd::Server { cmd: Some(ServerCmd::Members { server, filter, limit }) } => {
            let v = payload_call("members", json!({ "guild": server, "query": filter, "limit": limit }))?;
            out(json, &v, || render::members(&v));
            Ok(())
        }
        Cmd::Channel { cmd: ChannelCmd::List { server } } => {
            let v = payload_call("channels", json!({ "guild": server }))?;
            out(json, &v, || render::channels(&v));
            Ok(())
        }
        Cmd::Chat { cmd: None | Some(ChatCmd::List) } => {
            let v = payload_call("dms", json!({}))?;
            out(json, &v, || render::chats(&v));
            Ok(())
        }
        Cmd::Friend { cmd: None | Some(FriendCmd::List) } => {
            let v = payload_call("friends", json!({}))?;
            out(json, &v, || render::friends(&v));
            Ok(())
        }
        Cmd::Read(a) => message_read(a, json),
        Cmd::Send(a) => message_send(a, json),
        Cmd::Message { cmd } => match cmd {
            MessageCmd::Read(a) => message_read(a, json),
            MessageCmd::Send(a) => message_send(a, json),
            MessageCmd::Edit { target, message, text, stdin, mentions, dry_run } => {
                let content = api::message_text(&text, stdin)?;
                let v = payload_call(
                    "edit",
                    json!({ "channel": target, "message": message, "content": content, "mentions": mentions, "dry_run": dry_run }),
                )?;
                out(json, &v, || render::write_result(&v));
                Ok(())
            }
            MessageCmd::Delete { target, message, dry_run } => {
                let v = payload_call("delete", json!({ "channel": target, "message": message, "dry_run": dry_run }))?;
                out(json, &v, || render::write_result(&v));
                Ok(())
            }
            MessageCmd::React { target, message, emoji, remove, dry_run } => {
                let v = payload_call(
                    "react",
                    json!({ "channel": target, "message": message, "emoji": emoji, "remove": remove, "dry_run": dry_run }),
                )?;
                out(json, &v, || render::write_result(&v));
                Ok(())
            }
        },
        Cmd::Voice { cmd: VoiceCmd::Who { target, server } } => {
            if target.is_none() && server.is_none() {
                bail!("say which voice channel (`discordctl voice who Server/channel`) or `--server <name>`");
            }
            let v = payload_call("voice_who", json!({ "channel": target, "guild": server }))?;
            out(json, &v, || render::voice_who(&v));
            Ok(())
        }
        Cmd::Voice { cmd } => {
            let (method, params) = match cmd {
                VoiceCmd::Status => ("voice_status", json!({})),
                VoiceCmd::Who { .. } => unreachable!("handled above"),
                VoiceCmd::Join { target, ring, dry_run } => {
                    ("voice_join", json!({ "channel": target, "ring": ring, "dry_run": dry_run }))
                }
                VoiceCmd::Leave { dry_run } => ("voice_leave", json!({ "dry_run": dry_run })),
                VoiceCmd::Mute => ("voice_mute", json!({ "mute": true })),
                VoiceCmd::Unmute => ("voice_mute", json!({ "mute": false })),
                VoiceCmd::Deafen => ("voice_deafen", json!({ "deafen": true })),
                VoiceCmd::Undeafen => ("voice_deafen", json!({ "deafen": false })),
            };
            let v = payload_call(method, params)?;
            out(json, &v, || render::voice(&v));
            Ok(())
        }
        Cmd::Stream { cmd } => {
            match cmd {
                StreamCmd::List => {
                    let v = payload_call("stream_sources", json!({}))?;
                    out(json, &v, || render::stream_sources(&v));
                }
                other => {
                    let v = match other {
                        StreamCmd::Status => payload_call("stream_status", json!({}))?,
                        StreamCmd::Start { source, audio, dry_run } => {
                            let id = resolve_source(&source)?;
                            payload_call("stream_start", json!({ "source": id, "audio": audio, "dry_run": dry_run }))?
                        }
                        StreamCmd::Stop { dry_run } => payload_call("stream_stop", json!({ "dry_run": dry_run }))?,
                        StreamCmd::List => unreachable!(),
                    };
                    out(json, &v, || render::stream(&v));
                }
            }
            Ok(())
        }

        Cmd::App { cmd } => app(cmd, json),
        Cmd::Mcp { cmd } => match cmd {
            McpCmd::Install { agent, preset, yes: _ } => mcp::install(agent, preset, json),
            McpCmd::Status => mcp::status(json),
            McpCmd::Remove { agent } => mcp::remove(agent, json),
        },
        Cmd::Daemon { cmd } => match cmd {
            DaemonCmd::Start => {
                client::ensure_daemon(T).map_err(anyhow::Error::msg)?;
                out(json, &json!({ "running": true }), || ui::ok("daemon is running"));
                Ok(())
            }
            DaemonCmd::Stop { force } => {
                api::stop_daemon(force)?;
                out(json, &json!({ "running": false }), || ui::ok("daemon stopped"));
                Ok(())
            }
        },
        Cmd::Completions { shell } => {
            let shell = match shell {
                Shell::Powershell => clap_complete::Shell::PowerShell,
                Shell::Bash => clap_complete::Shell::Bash,
                Shell::Zsh => clap_complete::Shell::Zsh,
                Shell::Fish => clap_complete::Shell::Fish,
            };
            clap_complete::generate(shell, &mut Cli::command(), "discordctl", &mut std::io::stdout());
            Ok(())
        }
        Cmd::Dev { cmd } => match cmd {
            DevCmd::Call { method, params } => {
                let params = match params {
                    Some(p) => serde_json::from_str(&p).context("params must be valid JSON")?,
                    None => json!({}),
                };
                let data = payload_call(&method, params)?;
                println!("{}", serde_json::to_string_pretty(&data)?);
                Ok(())
            }
            DevCmd::Reload { payload } => reload_payload(payload, json),
        },
    }
}

/// `discordctl lang [en|ru|auto]`: показать или сохранить язык интерфейса.
fn language(value: Option<String>, json: bool) -> Result<()> {
    use i18n::{Choice, Lang};
    if let Some(v) = value {
        let Some(choice) = Choice::parse(&v) else { bail!("unknown language {v:?}: use en, ru or auto") };
        i18n::save(choice).context("saving the language")?;
        i18n::set(choice.resolve());
        out(
            json,
            &json!({ "lang": choice.word(), "effective": if i18n::current() == Lang::Ru { "ru" } else { "en" } }),
            || ui::ok(format!("language set to {}", choice.word())),
        );
        return Ok(());
    }
    let (saved, now) = (i18n::saved(), i18n::current());
    let name = if now == Lang::Ru { i18n::tr("Russian").into_owned() } else { "English".to_owned() };
    out(json, &json!({ "lang": saved.word(), "effective": if now == Lang::Ru { "ru" } else { "en" } }), || {
        ui::info(format!("language: {} ({})", saved.word(), name));
        ui::hint("Change it with `discordctl lang ru|en|auto` (or --lang, or DISCORDCTL_LANG).");
    });
    Ok(())
}

fn message_read(a: ReadArgs, json: bool) -> Result<()> {
    let v = payload_call(
        "messages",
        json!({ "channel": a.target, "limit": a.limit, "before": a.before, "after": a.after }),
    )?;
    out(json, &v, || render::messages(&v));
    Ok(())
}

fn message_send(a: SendArgs, json: bool) -> Result<()> {
    let content = api::message_text(&a.text, a.stdin)?;
    let v = payload_call(
        "send",
        json!({ "channel": a.target, "content": content, "reply_to": a.reply_to, "mentions": a.mentions, "dry_run": a.dry_run }),
    )?;
    out(json, &v, || render::write_result(&v));
    Ok(())
}

/// Источник по id (`window:…`, `screen:…`) или по части названия окна. Неоднозначное название — ошибка со списком.
fn resolve_source(arg: &str) -> Result<String> {
    if arg.starts_with("window:") || arg.starts_with("screen:") {
        return Ok(arg.to_owned());
    }
    // У приложения список источников может быть закрыт: тогда передаём как есть, ответит демон.
    let Ok(list) = payload_call("stream_sources", json!({})) else { return Ok(arg.to_owned()) };
    let q = arg.to_lowercase();
    let hits: Vec<&Value> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter(|x| x["name"].as_str().is_some_and(|n| n.to_lowercase().contains(&q)))
        .collect();
    match hits.as_slice() {
        [one] => Ok(one["id"].as_str().unwrap_or("").to_owned()),
        [] => bail!("no window or screen matches {arg:?} (see `discordctl stream list`)"),
        many => bail!(
            "{arg:?} matches several windows: {}",
            many.iter()
                .map(|x| format!("\"{}\" ({})", x["name"].as_str().unwrap_or("?"), x["id"].as_str().unwrap_or("?")))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn reload_payload(payload: Option<std::path::PathBuf>, json: bool) -> Result<()> {
    let payload_path = payload
        .map(|p| {
            std::fs::canonicalize(&p)
                .map(|c| c.to_string_lossy().trim_start_matches(r"\\?\").to_owned())
                .with_context(|| format!("{}", p.display()))
        })
        .transpose()?;
    owner_call(&Request::HookReload { payload_path })?;
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Some(h) = ask(&Request::Status)?.and_then(|d| d.data).map(|d| d["hook"].clone()) {
            if h["active"] == true {
                out(json, &h, || ui::ok(format!("payload active, {} stores", h["payload"]["stores"])));
                return Ok(());
            }
        }
    }
    bail!("the payload did not become active in 30 s (see `discordctl status`)")
}

fn status(json: bool) -> Result<()> {
    let cfg = Config::load();
    let root = cfg.root();
    let daemon = ask(&Request::Status)?;
    let data = daemon.as_ref().and_then(|d| d.data.clone());
    let v = json!({ "daemon": data, "daemon_running": daemon.is_some(), "shim": api::shim_status_value(&root) });
    out(json, &v, || {
        match &data {
            Some(d) => ui::ok(format!(
                "daemon running {}",
                ui::dim(format!("(v{}, pid {})", d["version"].as_str().unwrap_or("?"), d["daemon_pid"]))
            )),
            None => ui::info("daemon not running (it starts with Discord)"),
        }
        match data.as_ref().map(|d| &d["managed_discord"]).filter(|m| !m.is_null()) {
            Some(m) => ui::ok(format!("Discord is attached {}", ui::dim(format!("(pid {})", m["pid"])))),
            None => ui::info("Discord is not attached to discordctl"),
        }
        if let Some(h) = data.as_ref().map(|d| &d["hook"]) {
            if h["active"] == true {
                ui::ok(format!("connection active {}", ui::dim(format!("({} stores)", h["payload"]["stores"]))));
            } else if let Some(p) = h["phase"].as_str().filter(|p| *p != "idle") {
                ui::warn(format!("connection {p} {}", h["error"].as_str().unwrap_or("")));
            }
        }
        let (u, s) = (shim::state(&root, Target::Update), shim::state(&root, Target::Stub));
        let mark = |st: shim::State| i18n::tr(&format!("{st:?}")).into_owned();
        ui::info(format!("launcher: Update.exe {}, Discord.exe {}", mark(u), mark(s)));
    });
    Ok(())
}

// ---- приложения ----------------------------------------------------------------------------------------------

fn app(cmd: AppCmd, json: bool) -> Result<()> {
    match cmd {
        AppCmd::Add(a) => {
            let (name, desc, perms) = match &a.manifest {
                Some(path) => {
                    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
                    let m = Manifest::parse(&text).map_err(anyhow::Error::msg)?;
                    (a.name.clone().unwrap_or(m.app.name), m.app.description, m.permissions)
                }
                None => {
                    let Some(name) = a.name.clone() else {
                        bail!("give the app a name: `discordctl app add <name> --read @alice …` (or --manifest <file>)")
                    };
                    let mut p = Permissions {
                        list: a.list,
                        read: a.read,
                        write: a.write,
                        voice: a.voice,
                        voice_channels: a.voice_channel,
                        stream: a.stream,
                        confirm: if a.no_confirm { Some(vec![]) } else { None },
                    };
                    p.normalize();
                    (name, None, p)
                }
            };
            let Some(final_perms) = approve::review(&name, desc.as_deref(), &perms)? else { bail!("not granted") };
            let r = owner_call(&Request::AuthGrant { name: name.clone(), permissions: final_perms })?;
            let token = r.data.as_ref().and_then(|d| d["token"].as_str()).unwrap_or("").to_owned();
            out(json, &json!({ "app": name, "token": token }), || {
                ui::ok(format!("app {} created", ui::bold(&name)));
                ui::say(format!(
                    "\nThe token is shown once. Give it to the app in the {} environment variable:\n\n  {}\n",
                    ui::bold("DISCORDCTL_TOKEN"),
                    ui::green(&token)
                ));
                ui::hint(format!("Revoke any time: discordctl app remove {name}"));
            });
            Ok(())
        }
        AppCmd::List => {
            let v = owner_call(&Request::AuthList)?.data.unwrap_or_default();
            out(json, &v, || {
                let apps = v.as_array().cloned().unwrap_or_default();
                if apps.is_empty() {
                    return ui::info("no apps yet: `discordctl mcp install` or `discordctl app add`");
                }
                let sum = |p: &Value, k: &str| {
                    let a = p[k].as_array().map(|a| a.len()).unwrap_or(0);
                    if a == 0 {
                        ui::dim("–")
                    } else {
                        a.to_string()
                    }
                };
                let rows: Vec<Vec<String>> = apps
                    .iter()
                    .map(|a| {
                        let p = &a["permissions"];
                        vec![
                            ui::bold(a["name"].as_str().unwrap_or("?")),
                            sum(p, "list"),
                            sum(p, "read"),
                            sum(p, "write"),
                            sum(p, "voice"),
                            sum(p, "stream"),
                        ]
                    })
                    .collect();
                println!("{}", ui::table(&["APP", "LIST", "READ", "WRITE", "VOICE", "SHARE"], &rows));
                ui::hint("Numbers are how many rules each right has. `discordctl app list --json` shows the details.");
            });
            Ok(())
        }
        AppCmd::Remove { name } => {
            owner_call(&Request::AuthRevoke { name: name.clone() })?;
            out(json, &json!({ "revoked": name }), || ui::ok(format!("{name}: token and rights revoked")));
            Ok(())
        }
        AppCmd::Log { limit } => {
            let v = owner_call(&Request::AuthAudit { limit })?.data.unwrap_or_default();
            out(json, &v, || {
                let rows: Vec<Vec<String>> = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| {
                        let d = e["decision"].as_str().unwrap_or("");
                        let colored = match d {
                            "allowed" | "granted" => ui::green(d),
                            "denied" | "error" => ui::red(d),
                            _ => ui::yellow(d),
                        };
                        vec![
                            ui::dim(e["ts"].to_string()),
                            ui::bold(e["app"].as_str().unwrap_or("")),
                            e["method"].as_str().unwrap_or("").to_owned(),
                            colored,
                            e["resource"].as_str().unwrap_or("").to_owned(),
                            ui::dim(e["detail"].as_str().unwrap_or("")),
                        ]
                    })
                    .collect();
                if rows.is_empty() {
                    ui::info("the audit log is empty");
                } else {
                    println!("{}", ui::table(&["TIME", "APP", "ACTION", "RESULT", "TARGET", "DETAIL"], &rows));
                }
            });
            Ok(())
        }
        AppCmd::Approve => approve::approvals_loop(),
        AppCmd::Whoami => {
            let r = owner_call(&Request::Whoami)?;
            out(json, &serde_json::to_value(&r)?, || match &r.data {
                Some(d) => ui::info(format!("app {} with {}", d["app"], d["permissions"])),
                None => ui::info("owner (no token)"),
            });
            Ok(())
        }
    }
}
