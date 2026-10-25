//! `discordctl setup`: установка по шагам с вопросами, затем подключение ИИ-агента.

use crate::{
    api::{ask, owner_call, stop_daemon},
    cli::SetupArgs,
    doctor, mcp, ops, ui,
};
use anyhow::{bail, Result};
use discordctl_core::{config::Config, paths, protocol::Request};
use std::time::{Duration, Instant};

/// Ждёт, пока хук в Discord станет активным.
fn wait_hook(spinner: &mut ui::Spinner, secs: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Ok(Some(r)) = ask(&Request::Status) {
            let hook = r.data.map(|d| d["hook"].clone()).unwrap_or_default();
            match hook["phase"].as_str().unwrap_or("") {
                "active" => return true,
                p @ ("waiting_page" | "installing" | "waiting_payload") => {
                    spinner.update(format!("Discord is loading ({p})"))
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(700));
    }
    false
}

pub fn run(a: SetupArgs) -> Result<()> {
    ui::banner();
    let yes = a.yes;
    if !yes && !ui::interactive() {
        bail!("setup asks questions and needs an interactive terminal; add --yes to accept the defaults (or use `discordctl install`)");
    }
    let root = a.root.clone().unwrap_or_else(paths::default_discord_root);

    // --- 1. проверка
    ui::heading("1. Checking");
    match paths::latest_app_dir(&root) {
        Some(dir) => ui::ok(format!("Discord found: {}", ui::dim(dir.display().to_string()))),
        None => {
            ui::fail(format!("Discord is not installed in {}", root.display()));
            ui::hint("Install the desktop app from discord.com first (the browser version cannot be controlled), or pass --root <folder>.");
            bail!("Discord not found");
        }
    }
    let was_installed = Config::load().shim_installed;
    if was_installed {
        ui::info("discordctl is already installed: files will be refreshed");
    }
    let running = ops::discord_running(&root);

    println!();
    println!("discordctl will:");
    println!("  {} copy its programs to {}", ui::cyan("•"), ui::dim(paths::bin_dir().display().to_string()));
    println!(
        "  {} put a small launcher in front of {} and {} (originals stay next to them as *.real.exe)",
        ui::cyan("•"),
        ui::bold("Update.exe"),
        ui::bold("Discord.exe")
    );
    println!("  {}", ui::dim("Everything is reverted by `discordctl uninstall`. The launcher always falls back to the real Discord if something fails."));
    if !yes && !ui::confirm("Continue?", true)? {
        bail!("cancelled");
    }
    // Запись в реестр чужого протокола антивирусы не любят, поэтому по умолчанию её нет. `None` оставляет прежний выбор.
    let link_hook = if yes {
        None
    } else {
        let already = Config::load().handler_hook;
        Some(ui::confirm("Also point discord:// links at the launcher? (edits one registry value; optional)", already)?)
    };

    // --- 2. установка
    ui::heading("2. Installing");
    // Работающий управляемый Discord держит пайп демона: чтобы обновить демон, Discord придётся перезапустить.
    let managed = ask(&Request::Status)?.and_then(|r| r.data).is_some_and(|d| !d["managed_discord"].is_null());
    let mut restarted = false;
    if running {
        let must_restart = managed;
        let want = !a.no_restart
            && (yes
                || ui::confirm(
                    if must_restart {
                        "Discord has to be restarted to update the daemon (calls will be interrupted). Close it now?"
                    } else {
                        "Discord is running without discordctl. Restart it so discordctl can attach? (calls will be interrupted)"
                    },
                    true,
                )?);
        if want {
            let sp = ui::Spinner::start("closing Discord");
            let _ = stop_daemon(true); // без демона команда ничего не делает
            let ok = ops::close_discord(&root);
            sp.finish_with(ok, if ok { "Discord closed" } else { "could not close Discord" });
            if !ok {
                bail!("close Discord manually (also from the tray) and run setup again");
            }
            restarted = true;
        } else if managed {
            bail!("a running Discord blocks the daemon update: close it, or rerun without --no-restart");
        }
    }
    let mut sp = ui::Spinner::start("installing");
    let mut step = |s: &str| sp.update(s.to_owned());
    let result = ops::install_with(root, link_hook, &mut step);
    match result {
        Ok(r) => sp.done(format!(
            "installed {}",
            ui::dim(format!("(discord:// links: {})", r["link_handler"].as_str().unwrap_or("?")))
        )),
        Err(e) => {
            sp.failed("installation failed");
            return Err(e);
        }
    }
    if ops::path_managed() && !ops::bin_on_path() {
        let add = yes || ui::confirm("Add discordctl to your PATH so it works from any terminal?", true)?;
        if add {
            match ops::add_bin_to_path() {
                Ok(_) => ui::ok("added to PATH (open a new terminal to use `discordctl` anywhere)"),
                Err(e) => ui::warn(format!("could not change PATH: {e:#}")),
            }
        }
    }

    // --- 3. запуск
    ui::heading("3. Starting Discord");
    if restarted || !running {
        let mut sp = ui::Spinner::start("starting Discord under discordctl");
        match owner_call(&Request::Launch { args: vec![] }) {
            Ok(_) => {
                // Для тестов срок можно сократить: поддельному Discord соединяться нечем.
                let wait = std::env::var("DISCORDCTL_SETUP_WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(90);
                let ok = wait_hook(&mut sp, wait);
                sp.finish_with(
                    ok,
                    if ok {
                        "connected to Discord"
                    } else {
                        "Discord started, but the connection is not ready yet (see doctor below)"
                    },
                );
            }
            Err(e) => sp.failed(format!("could not start Discord: {e:#}")),
        }
    } else {
        ui::warn(
            "Discord is still running without discordctl: close it completely (also from the tray) and start it again",
        );
    }

    // --- 4. проверка
    ui::heading("4. Health check");
    doctor::run(false, false)?;

    // --- 5. агент
    ui::heading("5. AI agent");
    if !a.agent.is_empty() {
        mcp::install(a.agent.clone(), a.preset, false)?;
    } else if !yes && ui::confirm("Connect an AI agent (Claude Code, Claude Desktop, Cursor) now?", true)? {
        mcp::install(vec![], a.preset, false)?;
    } else {
        ui::info(format!("skipped. Later: {}", ui::bold("discordctl mcp install")));
    }

    println!("\n{}", ui::bold("All set. Try:"));
    println!("  {}   list your chats", ui::cyan("discordctl chat"));
    println!("  {}   read the last messages", ui::cyan("discordctl read @name -n 20"));
    println!("  {}   check the installation any time\n", ui::cyan("discordctl doctor"));
    Ok(())
}
