//! Разбор аргументов шима и белый список аргументов запуска Discord.
//!
//! Белый список нужен потому, что `launch` доступен любому процессу пользователя через pipe.
//! Без него такой процесс мог бы запустить Discord с `--remote-debugging-port` и обойти нашу авторизацию.

use std::ffi::OsString;

/// Если аргументы шима означают «запусти Discord» (`--processStart Discord.exe [--process-start-args ...]`),
/// возвращает аргументы для самого Discord. Всё остальное (установка, обновление) шим передаёт настоящему Update.exe.
pub fn parse_process_start(args: &[OsString]) -> Option<Vec<String>> {
    let a: Vec<String> = args.iter().map(|s| s.to_string_lossy().into_owned()).collect();
    let first = a.first()?.to_ascii_lowercase();
    let rest: &[String] = if first == "--processstart" || first == "-processstart" {
        if !a.get(1)?.eq_ignore_ascii_case("discord.exe") {
            return None;
        }
        &a[2..]
    } else if first == "--processstart=discord.exe" || first == "-processstart=discord.exe" {
        &a[1..]
    } else {
        return None;
    };
    match rest.first() {
        None => Some(vec![]),
        Some(f) if f.eq_ignore_ascii_case("--process-start-args") => {
            Some(rest[1..].join(" ").split_whitespace().map(str::to_owned).collect())
        }
        Some(_) => None,
    }
}

/// Для шима на месте корневой заглушки `Discord.exe`: любой запуск с безопасными аргументами
/// (включая пустые и `--processStart Discord.exe`) считается запуском Discord.
pub fn parse_stub_launch(args: &[OsString]) -> Option<Vec<String>> {
    if let Some(extra) = parse_process_start(args) {
        return Some(extra);
    }
    let a: Vec<String> = args.iter().map(|s| s.to_string_lossy().into_owned()).collect();
    sanitize_launch_args(&a).ok()
}

const MAX_URL: usize = 2048;

fn url_ok(s: &str) -> bool {
    s.len() <= MAX_URL
        && s.starts_with("discord://")
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c))
}

/// Пропускает только безопасные аргументы: `--start-minimized`, `--url`, `--` и ссылки `discord://`.
pub fn sanitize_launch_args(tokens: &[String]) -> Result<Vec<String>, String> {
    for t in tokens {
        let ok = matches!(t.as_str(), "--start-minimized" | "--url" | "--") || url_ok(t);
        if !ok {
            return Err(format!("argument not allowed: {t}"));
        }
    }
    Ok(tokens.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn plain_process_start() {
        assert_eq!(parse_process_start(&os(&["--processStart", "Discord.exe"])), Some(vec![]));
        assert_eq!(parse_process_start(&os(&["--processstart", "discord.exe"])), Some(vec![]));
        assert_eq!(parse_process_start(&os(&["--processStart=Discord.exe"])), Some(vec![]));
    }

    #[test]
    fn process_start_with_args() {
        let got = parse_process_start(&os(&[
            "--processStart",
            "Discord.exe",
            "--process-start-args",
            "--url -- discord://x/y",
        ]));
        assert_eq!(got, Some(vec!["--url".into(), "--".into(), "discord://x/y".into()]));
    }

    #[test]
    fn everything_else_is_forwarded() {
        assert_eq!(parse_process_start(&os(&["--update", "https://x"])), None);
        assert_eq!(parse_process_start(&os(&["--processStart", "Other.exe"])), None);
        assert_eq!(parse_process_start(&os(&["--processStart", "Discord.exe", "--weird"])), None);
        assert_eq!(parse_process_start(&os(&[])), None);
    }

    #[test]
    fn stub_treats_safe_invocations_as_launch() {
        assert_eq!(parse_stub_launch(&os(&[])), Some(vec![]));
        assert_eq!(parse_stub_launch(&os(&["--processStart", "Discord.exe"])), Some(vec![]));
        assert_eq!(parse_stub_launch(&os(&["--start-minimized"])), Some(vec!["--start-minimized".into()]));
        assert_eq!(parse_stub_launch(&os(&["--url", "--", "discord://-/a"])).map(|v| v.len()), Some(3));
    }

    #[test]
    fn stub_forwards_anything_unknown() {
        assert_eq!(parse_stub_launch(&os(&["--remote-debugging-port=1"])), None);
        assert_eq!(parse_stub_launch(&os(&["--weird"])), None);
        assert_eq!(parse_stub_launch(&os(&["--processStart", "Other.exe"])), None);
    }

    #[test]
    fn debug_switches_are_rejected() {
        assert!(sanitize_launch_args(&["--remote-debugging-port=9222".into()]).is_err());
        assert!(sanitize_launch_args(&["--inspect".into()]).is_err());
        assert!(sanitize_launch_args(&["discord://a\" --remote-debugging-port=1".into()]).is_err());
        assert!(sanitize_launch_args(&["https://evil".into()]).is_err());
    }

    #[test]
    fn deep_link_is_allowed() {
        let t: Vec<String> = ["--url", "--", "discord://-/channels/1/2"].iter().map(|s| s.to_string()).collect();
        assert_eq!(sanitize_launch_args(&t).unwrap(), t);
        assert!(sanitize_launch_args(&["--start-minimized".into()]).is_ok());
    }
}
