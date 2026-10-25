//! Оформление вывода: цвета, значки, таблицы, спиннер и вопросы.
//!
//! Правила: без терминала или с `NO_COLOR`/`--no-color` цвета выключены, спиннер печатает строки, а вопросы отказывают
//! с понятным сообщением (флаги заменяют ответы). Машинный вывод (`--json`) этот модуль не трогает.

use crate::i18n::tr;
use anyhow::{bail, Result};
use console::{measure_text_width, style, Term};
use dialoguer::{theme::ColorfulTheme, Confirm, MultiSelect};
use indicatif::{ProgressBar, ProgressStyle};
use std::{io::IsTerminal, time::Duration};

pub fn init(no_color: bool) {
    if no_color {
        console::set_colors_enabled(false);
        console::set_colors_enabled_stderr(false);
    }
}

pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

// Все тексты проходят через `tr`: на английском он ничего не меняет, на русском подставляет перевод (см. `i18n.rs`).
pub fn ok(msg: impl AsRef<str>) {
    println!("{} {}", style("√").green().bold(), tr(msg.as_ref()));
}
pub fn warn(msg: impl AsRef<str>) {
    println!("{} {}", style("!").yellow().bold(), tr(msg.as_ref()));
}
pub fn fail(msg: impl AsRef<str>) {
    println!("{} {}", style("×").red().bold(), tr(msg.as_ref()));
}
pub fn info(msg: impl AsRef<str>) {
    println!("{} {}", style("•").cyan(), tr(msg.as_ref()));
}
/// Подсказка под предыдущей строкой.
pub fn hint(msg: impl AsRef<str>) {
    let msg = tr(msg.as_ref());
    for line in msg.lines() {
        println!("  {} {}", style("→").dim(), style(line).dim());
    }
}
pub fn heading(title: &str) {
    println!("\n{}", style(tr(title)).bold().underlined());
}
/// Печатает уже собранную строку (с подставленными значениями), переведя её целиком.
pub fn say(line: impl AsRef<str>) {
    println!("{}", tr(line.as_ref()));
}
pub fn dim(s: impl AsRef<str>) -> String {
    style(tr(s.as_ref())).dim().to_string()
}
pub fn bold(s: impl AsRef<str>) -> String {
    style(tr(s.as_ref())).bold().to_string()
}
pub fn cyan(s: impl AsRef<str>) -> String {
    style(tr(s.as_ref())).cyan().to_string()
}
pub fn green(s: impl AsRef<str>) -> String {
    style(tr(s.as_ref())).green().to_string()
}
pub fn yellow(s: impl AsRef<str>) -> String {
    style(tr(s.as_ref())).yellow().to_string()
}
pub fn red(s: impl AsRef<str>) -> String {
    style(tr(s.as_ref())).red().to_string()
}

pub fn banner() {
    let t = style("discordctl").bold().cyan();
    println!("\n  {t}  {}", style(tr("your Discord, from the terminal and your AI agents")).dim());
    println!(
        "  {}\n",
        style(tr("Windows desktop client · automation breaks Discord's terms: use at your own risk")).dim()
    );
}

/// Таблица с приглушённой шапкой; ширина считается по видимым символам (кириллица, цвета).
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let cols = headers.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let mut widths = vec![0usize; cols];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = measure_text_width(&tr(h));
    }
    for r in rows {
        for (i, cell) in r.iter().enumerate() {
            widths[i] = widths[i].max(measure_text_width(cell));
        }
    }
    let pad = |cell: &str, w: usize, last: bool| {
        if last {
            cell.to_owned()
        } else {
            format!("{cell}{}", " ".repeat(w.saturating_sub(measure_text_width(cell))))
        }
    };
    let mut out = Vec::new();
    if !headers.is_empty() {
        let line: Vec<String> = headers
            .iter()
            .enumerate()
            .map(|(i, h)| style(pad(&tr(h), widths[i], i + 1 == headers.len())).dim().to_string())
            .collect();
        out.push(line.join("  "));
    }
    for r in rows {
        let line: Vec<String> = r.iter().enumerate().map(|(i, c)| pad(c, widths[i], i + 1 == r.len())).collect();
        out.push(line.join("  "));
    }
    out.join("\n")
}

/// Спиннер для долгих шагов. Без терминала печатает обычные строки.
pub struct Spinner {
    bar: Option<ProgressBar>,
    msg: String,
}

impl Spinner {
    pub fn start(msg: impl Into<String>) -> Spinner {
        let msg = tr(&msg.into()).into_owned();
        if Term::stdout().is_term() && console::colors_enabled() {
            let bar = ProgressBar::new_spinner();
            bar.set_style(
                ProgressStyle::with_template("{spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "√"]),
            );
            bar.set_message(msg.clone());
            bar.enable_steady_tick(Duration::from_millis(80));
            Spinner { bar: Some(bar), msg }
        } else {
            println!("… {msg}");
            Spinner { bar: None, msg }
        }
    }
    pub fn update(&mut self, msg: impl Into<String>) {
        self.msg = tr(&msg.into()).into_owned();
        if let Some(b) = &self.bar {
            b.set_message(self.msg.clone());
        }
    }
    fn end(self, line: String) {
        match self.bar {
            Some(b) => {
                b.finish_and_clear();
                println!("{line}");
            }
            None => println!("{line}"),
        }
    }
    pub fn done(self, msg: impl AsRef<str>) {
        let line = format!("{} {}", style("√").green().bold(), tr(msg.as_ref()));
        self.end(line);
    }
    pub fn failed(self, msg: impl AsRef<str>) {
        let line = format!("{} {}", style("×").red().bold(), tr(msg.as_ref()));
        self.end(line);
    }
    /// Завершить и перейти к следующему шагу с тем же текстом.
    pub fn finish_with(self, ok: bool, msg: impl AsRef<str>) {
        if ok {
            self.done(msg)
        } else {
            self.failed(msg)
        }
    }
}

fn need_terminal(what: &str) -> Result<()> {
    if !interactive() {
        bail!("{} needs an interactive terminal; pass the answers as flags (see --help)", tr(what));
    }
    Ok(())
}

pub fn confirm(question: &str, default: bool) -> Result<bool> {
    need_terminal("a question")?;
    Ok(Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(tr(question).into_owned())
        .default(default)
        .interact()?)
}

pub fn multiselect(question: &str, items: &[&str], defaults: &[bool]) -> Result<Vec<usize>> {
    need_terminal("a choice")?;
    Ok(MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("{} {}", tr(question), tr("(space to toggle, enter to confirm)")))
        .items(&items.iter().map(|i| tr(i).into_owned()).collect::<Vec<_>>())
        .defaults(defaults)
        .interact()?)
}

/// Ошибка верхнего уровня: красная строка и (если есть) цепочка причин.
pub fn print_error(e: &anyhow::Error) {
    eprintln!("{} {}", style(tr("error:")).red().bold(), tr(&e.to_string()));
    for cause in e.chain().skip(1) {
        eprintln!("  {} {}", style(tr("caused by:")).dim(), tr(&cause.to_string()));
    }
}
