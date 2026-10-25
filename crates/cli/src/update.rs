//! `discordctl update`: узнать, есть ли новый релиз, скачать, проверить SHA-256 и установить.
//!
//! GitHub API не используется (60 запросов в час на IP): последний тег берётся из перенаправления страницы
//! `releases/latest`, файлы релиза лежат по предсказуемым адресам. Сумма проверяется по `SHA256SUMS.txt` того же
//! релиза: это защита от битой или подменённой при передаче загрузки, но не от подмены самого релиза на GitHub.

use crate::{
    api::{ask, stop_daemon},
    cli::UpdateArgs,
    http, ops, ui, zipread,
};
use anyhow::{bail, Context, Result};
use discordctl_core::{client, config::Config, paths, protocol::Request};
use serde_json::json;
use std::{cmp::Ordering, path::Path, process::Command, time::Duration};

const MAX_ZIP: usize = 64 * 1024 * 1024;
const MAX_SUMS: usize = 64 * 1024;
/// Файлы, без которых релиз не годится. `discordctl-mcp.exe` необязателен, как и при обычной установке.
const REQUIRED: [&str; 3] = ["discordctl.exe", "discordctl-shim.exe", "discordctl-daemon.exe"];

fn repo() -> String {
    std::env::var("DISCORDCTL_UPDATE_REPO")
        .ok()
        .filter(|r| r.split('/').count() == 2 && r.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)))
        .unwrap_or_else(|| env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/").to_owned())
}

/// Тег вида `v1.2.3`: только безопасные символы, он попадает в адрес и в имя папки.
fn clean_tag(tag: &str) -> Result<String> {
    let t = tag.trim();
    let t = if t.starts_with('v') { t.to_owned() } else { format!("v{t}") };
    let body = &t[1..];
    let ok = !body.is_empty()
        && body.len() <= 40
        && body.starts_with(|c: char| c.is_ascii_digit())
        && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if !ok {
        bail!("{tag:?} is not a version tag (expected something like v0.1.3)");
    }
    Ok(t)
}

/// `1.2.3` или `1.2.3-rc1` → (1, 2, 3). Хвост после `-` для сравнения не нужен.
fn version_key(v: &str) -> Option<Vec<u64>> {
    v.trim_start_matches('v').split('-').next()?.split('.').map(|p| p.parse().ok()).collect()
}

fn compare(a: &str, b: &str) -> Option<Ordering> {
    Some(version_key(a)?.cmp(&version_key(b)?))
}

/// Последний тег: адрес `releases/latest` отвечает перенаправлением на `releases/tag/<тег>`.
fn latest_tag(repo: &str) -> Result<String> {
    let r = http::get(&format!("https://github.com/{repo}/releases/latest"), false, MAX_SUMS)
        .context("cannot reach github.com")?;
    let tag = r.location.as_deref().and_then(|l| l.rsplit_once("/releases/tag/")).map(|(_, t)| t.to_owned());
    match (r.status, tag) {
        (300..=399, Some(t)) => clean_tag(t.split(['?', '#']).next().unwrap_or(&t)),
        // Без релизов страница перенаправляет на общий список, а не на тег.
        (404 | 300..=399, _) => bail!("{repo} has no published release yet"),
        (status, _) => bail!("github.com answered {status} to the release lookup"),
    }
}

fn fetch(url: &str, max: usize) -> Result<Vec<u8>> {
    let r = http::get(url, true, max).with_context(|| format!("downloading {url}"))?;
    if r.status != 200 {
        bail!("{url}: the server answered {}", r.status);
    }
    Ok(r.body)
}

/// Скачивает архив релиза и сверяет SHA-256 с `SHA256SUMS.txt` из того же релиза.
fn download(repo: &str, tag: &str) -> Result<Vec<u8>> {
    let name = format!("discordctl-{}-windows-x64.zip", tag.trim_start_matches('v'));
    let base = format!("https://github.com/{repo}/releases/download/{tag}");
    let sums = String::from_utf8_lossy(&fetch(&format!("{base}/SHA256SUMS.txt"), MAX_SUMS)?).into_owned();
    let zip = fetch(&format!("{base}/{name}"), MAX_ZIP)?;
    verify(&zip, &name, &sums)?;
    Ok(zip)
}

/// Сумма архива должна совпасть со строкой `<sha256>  <имя>` из файла сумм.
fn verify(zip: &[u8], name: &str, sums: &str) -> Result<()> {
    let expected = sums
        .lines()
        .find(|l| l.split_whitespace().nth(1).is_some_and(|n| n.trim_start_matches('*') == name))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_lowercase)
        .with_context(|| format!("SHA256SUMS.txt has no entry for {name}"))?;
    let actual = paths::sha256_hex(zip);
    if expected != actual {
        bail!("checksum mismatch for {name}: expected {expected}, got {actual}. Nothing was installed.");
    }
    Ok(())
}

/// Раскладывает программы из архива в `dir`.
fn unpack(zip: &[u8], dir: &Path) -> Result<()> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir)?;
    for name in ops::BIN_FILES {
        match zipread::read_root_file(zip, name)? {
            Some(bytes) => std::fs::write(dir.join(name), bytes).with_context(|| format!("writing {name}"))?,
            None if REQUIRED.contains(&name) => bail!("the release archive has no {name}"),
            None => {}
        }
    }
    Ok(())
}

pub fn run(a: UpdateArgs, json_mode: bool) -> Result<()> {
    let repo = repo();
    let current = env!("CARGO_PKG_VERSION");
    let tag = match &a.release {
        Some(v) => clean_tag(v)?,
        None => latest_tag(&repo)?,
    };
    let order = compare(&tag, current).context("cannot compare versions")?;
    let newer = order == Ordering::Greater;
    if a.check || (a.release.is_none() && !newer) {
        let v = json!({ "current": current, "latest": tag, "update_available": newer });
        if json_mode {
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else if newer {
            ui::info(format!("update available: v{current} → {tag}"));
            ui::hint("Install it with `discordctl update`.");
        } else {
            ui::ok(format!("up to date (v{current})"));
        }
        return Ok(());
    }
    if order == Ordering::Equal && !a.no_install {
        ui::info(format!("v{current} is already installed; reinstalling it from the release"));
    }

    let cfg = Config::load();
    let install = !a.no_install && cfg.shim_installed;
    if !a.yes && !json_mode {
        let verb = match order {
            Ordering::Less => "Install the OLDER version",
            _ if install => "Install",
            _ => "Download",
        };
        if !ui::confirm(&format!("{} {tag} (you have v{current})?", crate::i18n::tr(verb)), order != Ordering::Less)? {
            bail!("cancelled");
        }
    }

    let stage = paths::data_dir().join("updates").join(&tag);
    let sp = ui::Spinner::start(format!("downloading {tag}"));
    let unpacked = download(&repo, &tag).and_then(|zip| unpack(&zip, &stage));
    match &unpacked {
        Ok(()) => sp.done(format!("downloaded and verified {tag}")),
        Err(_) => sp.failed("download failed"),
    }
    unpacked?;

    if !install {
        let new_exe = stage.join("discordctl.exe");
        if json_mode {
            println!("{}", serde_json::to_string_pretty(&json!({ "downloaded": tag, "dir": stage }))?);
        } else if a.no_install {
            ui::ok(format!("unpacked to {}", stage.display()));
        } else {
            ui::warn("discordctl is not installed on this PC yet, so there is nothing to update");
            ui::hint(format!("Unpacked to {}. Run `{} setup` to install it.", stage.display(), new_exe.display()));
        }
        return Ok(());
    }

    let root = cfg.root();
    // Новый демон и новые файлы нельзя поставить, пока управляемый Discord держит пайп старого демона.
    let managed = ask(&Request::Status)?.and_then(|r| r.data).is_some_and(|d| !d["managed_discord"].is_null());
    if managed {
        if !a.yes
            && !ui::confirm(
                "Discord has to be closed to update the daemon (calls will be interrupted). Close it?",
                true,
            )?
        {
            bail!("cancelled: close Discord completely and run `discordctl update` again");
        }
        let _ = stop_daemon(true);
        if !ops::close_discord(&root) {
            bail!("could not close Discord: close it yourself (also from the tray) and run the update again");
        }
    }

    // Файлы кладём сами, до запуска скачанного установщика: наш exe может лежать в `bin/` и быть занят, а замену
    // занятых файлов (переименование в `.old`) умеем мы, но не обязательно скачанная версия (старые релизы её не умеют).
    stop_daemon(false).context("cannot stop the running daemon (close Discord first)")?;
    ops::copy_binaries_from(Some(&stage))?;
    // Дальше ставит новая копия: она знает, что нужно именно этой версии (шим, настройки, обработчик ссылок).
    let status = Command::new(stage.join("discordctl.exe"))
        .arg("install")
        .arg("--root")
        .arg(&root)
        .status()
        .context("running the downloaded installer")?;
    if !status.success() {
        bail!(
            "the downloaded installer failed (exit code {:?}); the program files are already replaced, run `discordctl doctor`",
            status.code()
        );
    }
    let _ = std::fs::remove_dir_all(&stage);

    if managed {
        match client::call(&Request::Launch { args: vec![] }, Duration::from_secs(10)) {
            Ok(r) if r.ok => ui::ok("Discord started again under discordctl"),
            Ok(r) => ui::warn(format!("Discord was not restarted: {}", r.msg.unwrap_or_default())),
            Err(e) => ui::warn(format!("Discord was not restarted: {e}")),
        }
    }
    ui::ok(format!("updated to {tag}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_checked_before_they_reach_an_address() {
        assert_eq!(clean_tag("0.1.3").unwrap(), "v0.1.3");
        assert_eq!(clean_tag("v1.0.0-rc1").unwrap(), "v1.0.0-rc1");
        for bad in ["", "v", "v../../x", "vx", "v1/2", "v1 2", "v1.0\"", "v1?x=1"] {
            assert!(clean_tag(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert_eq!(compare("v0.1.10", "0.1.9"), Some(Ordering::Greater));
        assert_eq!(compare("v0.1.2", "0.1.2"), Some(Ordering::Equal));
        assert_eq!(compare("v0.2.0", "0.10.0"), Some(Ordering::Less));
        assert_eq!(compare("v1.0.0-rc1", "0.9.9"), Some(Ordering::Greater));
        assert_eq!(compare("vabc", "0.1.0"), None);
    }

    #[test]
    fn checksum_must_match_the_entry_for_this_file() {
        let zip = b"archive bytes";
        let sum = paths::sha256_hex(zip);
        let name = "discordctl-0.1.3-windows-x64.zip";
        assert!(verify(zip, name, &format!("{sum}  {name}\n")).is_ok());
        assert!(verify(zip, name, &format!("{}  {name}\n", sum.to_uppercase())).is_ok(), "hex case does not matter");
        assert!(verify(zip, name, &format!("{sum}  other.zip\n")).is_err(), "a sum for another file is no proof");
        assert!(verify(b"tampered", name, &format!("{sum}  {name}\n")).is_err());
        assert!(verify(zip, name, "").is_err());
    }
}
