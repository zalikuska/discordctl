//! Язык интерфейса: английский (по умолчанию) и русский.
//!
//! Тексты в коде остаются английскими, они же служат ключами. Перевод подставляется в одном месте, там, где текст
//! выводится (`ui::*`, экраны мастера, вывод команд), поэтому вызовов `tr` в самой логике почти нет.
//!
//! Ключ в каталоге (`i18n_ru.rs`) — это строка из кода как есть. Места, куда подставляются значения (`{}`, `{name}`,
//! `{x:?}`), считаются любым текстом; в русском тексте они задаются так же (`{}` по порядку) или с номером
//! (`{2}`), если порядок слов другой. Строки, которых нет в каталоге, выводятся как есть.
//!
//! Выбор языка: `--lang`, затем `DISCORDCTL_LANG`, затем `discordctl lang` (сохраняется в config.json).
//! Значения: `en`, `ru`, `auto` (язык интерфейса Windows). Без настроек язык английский.

use clap::Command;
use discordctl_core::config::Config;
#[cfg(not(test))]
use std::sync::atomic::{AtomicU8, Ordering};
use std::{borrow::Cow, collections::HashMap, sync::OnceLock};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Ru,
}

#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(0);

// В тестах язык свой у каждого потока: тесты идут параллельно, и русский экран не должен портить английские.
#[cfg(test)]
thread_local! {
    static TEST_LANG: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

pub fn current() -> Lang {
    #[cfg(test)]
    let v = TEST_LANG.with(|c| c.get());
    #[cfg(not(test))]
    let v = CURRENT.load(Ordering::Relaxed);
    if v == 1 {
        Lang::Ru
    } else {
        Lang::En
    }
}

pub fn set(l: Lang) {
    let v = if l == Lang::Ru { 1 } else { 0 };
    #[cfg(test)]
    TEST_LANG.with(|c| c.set(v));
    #[cfg(not(test))]
    CURRENT.store(v, Ordering::Relaxed);
}

/// Что человек выбрал: язык или «как в системе».
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    En,
    Ru,
    Auto,
}

impl Choice {
    pub fn parse(s: &str) -> Option<Choice> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" | "eng" | "english" => Some(Choice::En),
            "ru" | "rus" | "russian" | "русский" => Some(Choice::Ru),
            "auto" | "system" => Some(Choice::Auto),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Choice::En => "en",
            Choice::Ru => "ru",
            Choice::Auto => "auto",
        }
    }

    pub fn resolve(self) -> Lang {
        match self {
            Choice::En => Lang::En,
            Choice::Ru => Lang::Ru,
            Choice::Auto if system_is_russian() => Lang::Ru,
            Choice::Auto => Lang::En,
        }
    }
}

/// Язык интерфейса Windows у пользователя русский.
fn system_is_russian() -> bool {
    // Основной язык LANGID: 0x19 — русский.
    let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    id & 0x3ff == 0x19
}

/// Выбор из аргументов (`--lang ru`, `--lang=ru`), окружения и настроек. Вызывается до разбора командной строки:
/// справка тоже печатается на выбранном языке.
pub fn detect(args: &[std::ffi::OsString]) -> Choice {
    let mut from_args = None;
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        let a = a.to_string_lossy();
        if let Some(v) = a.strip_prefix("--lang=") {
            from_args = Choice::parse(v);
        } else if a == "--lang" {
            from_args = it.next().and_then(|v| Choice::parse(&v.to_string_lossy()));
        }
    }
    from_args
        .or_else(|| std::env::var("DISCORDCTL_LANG").ok().and_then(|v| Choice::parse(&v)))
        .or_else(|| Config::load().lang.as_deref().and_then(Choice::parse))
        .unwrap_or(Choice::En)
}

/// Сохранённый выбор (без учёта аргументов и окружения).
pub fn saved() -> Choice {
    Config::load().lang.as_deref().and_then(Choice::parse).unwrap_or(Choice::En)
}

pub fn save(c: Choice) -> std::io::Result<()> {
    let mut cfg = Config::load();
    cfg.lang = (c != Choice::En).then(|| c.word().to_owned());
    cfg.save()
}

// ---- каталог -------------------------------------------------------------------------------------------------

enum Piece {
    Lit(String),
    Cap(usize),
}

struct Pattern {
    /// Постоянные куски английского текста между местами подстановки (их на один больше, чем подстановок).
    segs: Vec<String>,
    ru: Vec<Piece>,
    weight: usize,
}

struct Catalog {
    exact: HashMap<&'static str, &'static str>,
    patterns: Vec<Pattern>,
}

/// Разбирает текст на постоянные куски и подстановки. `{{` и `}}` — обычные скобки.
fn split_groups(s: &str) -> (Vec<String>, Vec<Option<usize>>) {
    let (mut segs, mut groups) = (vec![String::new()], vec![]);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                segs.last_mut().unwrap().push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                segs.last_mut().unwrap().push('}');
            }
            '{' => {
                let mut body = String::new();
                for d in chars.by_ref() {
                    if d == '}' {
                        break;
                    }
                    body.push(d);
                }
                // `{3}` — явный номер, всё остальное (`{}`, `{name}`, `{x:?}`) — следующая по порядку.
                groups.push(body.parse::<usize>().ok());
                segs.push(String::new());
            }
            c => segs.last_mut().unwrap().push(c),
        }
    }
    (segs, groups)
}

fn build() -> Catalog {
    let mut exact = HashMap::new();
    let mut patterns = vec![];
    for (en, ru) in crate::i18n_ru::RU.iter().chain(crate::i18n_ru_help::RU_HELP) {
        let (segs, groups) = split_groups(en);
        if groups.is_empty() {
            exact.insert(*en, *ru);
            continue;
        }
        let (rsegs, rgroups) = split_groups(ru);
        let mut next = 0;
        let mut ru_pieces = vec![];
        for (i, seg) in rsegs.into_iter().enumerate() {
            if !seg.is_empty() {
                ru_pieces.push(Piece::Lit(seg));
            }
            if let Some(g) = rgroups.get(i) {
                let n = g.map(|n| n - 1).unwrap_or_else(|| {
                    next += 1;
                    next - 1
                });
                ru_pieces.push(Piece::Cap(n));
            }
        }
        let weight = segs.iter().map(|s| s.len()).sum();
        patterns.push(Pattern { segs, ru: ru_pieces, weight });
    }
    // Сначала самые определённые: у них больше постоянного текста.
    patterns.sort_by_key(|p| std::cmp::Reverse(p.weight));
    Catalog { exact, patterns }
}

fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(build)
}

/// Подставляет в шаблон куски `s`, если он подходит.
fn capture<'a>(segs: &[String], s: &'a str) -> Option<Vec<&'a str>> {
    let n = segs.len();
    let first = &segs[0];
    let last = &segs[n - 1];
    if !s.starts_with(first.as_str()) || s.len() < first.len() + last.len() || !s.ends_with(last.as_str()) {
        return None;
    }
    let (mut pos, limit) = (first.len(), s.len() - last.len());
    let mut caps = vec![];
    for mid in &segs[1..n - 1] {
        let at = if mid.is_empty() { 0 } else { s[pos..limit].find(mid.as_str())? };
        caps.push(&s[pos..pos + at]);
        pos += at + mid.len();
        if pos > limit {
            return None;
        }
    }
    caps.push(&s[pos..limit]);
    Some(caps)
}

fn translate(s: &str) -> Option<String> {
    let c = catalog();
    if let Some(ru) = c.exact.get(s) {
        return Some((*ru).to_owned());
    }
    for p in &c.patterns {
        if let Some(caps) = capture(&p.segs, s) {
            let mut out = String::new();
            for piece in &p.ru {
                match piece {
                    Piece::Lit(t) => out.push_str(t),
                    Piece::Cap(i) => out.push_str(caps.get(*i).copied().unwrap_or("")),
                }
            }
            return Some(out);
        }
    }
    None
}

/// Перевод строки на выбранный язык; неизвестный текст возвращается как есть.
pub fn tr(s: &str) -> Cow<'_, str> {
    if current() == Lang::En || s.is_empty() {
        return Cow::Borrowed(s);
    }
    if let Some(t) = translate(s) {
        return Cow::Owned(t);
    }
    // Текст с пробелами по краям (отступ, колонка, дополненная до ширины): переводим слова и ставим пробелы обратно.
    let core = s.trim();
    if core.len() < s.len() && !core.is_empty() {
        if let Some(t) = translate(core) {
            let lead = &s[..s.len() - s.trim_start().len()];
            let had_trail = s.trim_end().len() < s.len();
            let mut out = format!("{lead}{t}");
            if had_trail {
                // Колонка сохраняет ширину, а если перевод длиннее, остаётся хотя бы один пробел.
                let width = s.chars().count();
                out.push_str(&" ".repeat(width.saturating_sub(out.chars().count()).max(1)));
            }
            return Cow::Owned(out);
        }
    }
    Cow::Borrowed(s)
}

/// Ширина колонки с подписью: по-русски слова длиннее.
pub fn col(en: usize) -> usize {
    if current() == Lang::Ru {
        en + 8
    } else {
        en
    }
}

/// Дополняет строку пробелами до `width` символов (кириллица считается по символам, не по байтам).
pub fn pad(s: &str, width: usize) -> String {
    let n = s.chars().count();
    format!("{s}{}", " ".repeat(width.saturating_sub(n).max(1)))
}

/// То же для `String`.
pub fn tr_owned(s: String) -> String {
    match tr(&s) {
        Cow::Borrowed(_) => s,
        Cow::Owned(t) => t,
    }
}

// ---- справка clap --------------------------------------------------------------------------------------------

/// Переводит справку: описания команд и параметров, заголовки разделов и общий обзор.
pub fn localize_command(mut cmd: Command) -> Command {
    cmd.build();
    localize(cmd, true)
}

fn localize(mut cmd: Command, top: bool) -> Command {
    if let Some(about) = cmd.get_about().map(|a| a.to_string()) {
        cmd = cmd.about(tr_owned(about));
    }
    if let Some(long) = cmd.get_long_about().map(|a| a.to_string()) {
        cmd = cmd.long_about(tr_owned(long));
    }
    if top {
        cmd = cmd.after_help(crate::i18n_ru::OVERVIEW_RU);
    }
    cmd = cmd
        .help_template("{about-with-newline}\nИспользование: {usage}\n\n{all-args}{after-help}")
        .subcommand_help_heading("Команды")
        .next_help_heading("Параметры");
    let ids: Vec<_> = cmd.get_arguments().map(|a| a.get_id().clone()).collect();
    for id in ids {
        cmd = cmd.mut_arg(id, |a| {
            // Встроенные `-h` и `-V` clap подписывает сам, по-английски.
            let a = match a.get_id().as_str() {
                "help" => a.help("Показать справку").long_help("Показать справку (краткую: -h)"),
                "version" => a.help("Показать версию"),
                _ => a,
            };
            let help = a.get_help().map(|h| h.to_string());
            let a = match help {
                Some(h) => a.help(tr_owned(h)),
                None => a,
            };
            if a.is_positional() {
                a.help_heading("Аргументы")
            } else {
                a.help_heading("Параметры")
            }
        });
    }
    let names: Vec<String> = cmd.get_subcommands().map(|s| s.get_name().to_owned()).collect();
    for name in names {
        cmd = cmd.mut_subcommand(name, |s| localize(s, false));
    }
    cmd
}

#[cfg(test)]
#[path = "i18n_tests.rs"]
mod tests;
