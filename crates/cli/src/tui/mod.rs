//! Полноэкранный интерфейс на ratatui: каркас экрана, цикл событий, фоновые задачи и виджеты выбора.
//!
//! Экраны мастера лежат в `wizard.rs`; здесь только то, что не зависит от содержимого. Терминал возвращается в
//! обычный режим при любом выходе, в том числе при панике.

pub mod wizard;

use crate::i18n::{self, tr, Lang};

use anyhow::{anyhow, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{
    collections::VecDeque,
    io::{stdout, Stdout},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

// ---- оформление ----------------------------------------------------------------------------------------------

pub const ACCENT: Color = Color::Rgb(114, 137, 255);
pub const OK: Color = Color::Rgb(87, 205, 130);
pub const WARN: Color = Color::Rgb(240, 190, 80);
pub const BAD: Color = Color::Rgb(240, 100, 100);
pub const DIM: Color = Color::Rgb(125, 130, 150);
const ROW_BG: Color = Color::Rgb(38, 42, 66);
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const MAX_WIDTH: u16 = 100;

pub fn fg(c: Color) -> Style {
    Style::new().fg(c)
}
pub fn bold(c: Color) -> Style {
    Style::new().fg(c).add_modifier(Modifier::BOLD)
}
pub fn dim() -> Style {
    fg(DIM)
}
pub fn spinner(tick: usize) -> &'static str {
    SPINNER[tick % SPINNER.len()]
}

/// Человек нажал Ctrl+C.
#[derive(Debug)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}
impl std::error::Error for Cancelled {}

/// Что сделал обработчик клавиш.
pub enum Flow<R> {
    Stay,
    Done(R),
}

/// Шаги вверху экрана.
pub const STEPS: [&str; 6] = ["Check", "Options", "Install", "Connect", "Agents", "Done"];
const STEPS_RU: [&str; 6] = ["Проверка", "Параметры", "Установка", "Связь", "Агенты", "Готово"];

/// Перевод всех кусков строки (экран перерисовывается каждый кадр, поэтому язык можно сменить на лету).
pub fn tr_span(s: Span<'static>) -> Span<'static> {
    let translated = tr(&s.content).into_owned();
    Span::styled(translated, s.style)
}

pub fn tr_line(l: Line<'static>) -> Line<'static> {
    let style = l.style;
    let alignment = l.alignment;
    let mut out = Line::from(l.spans.into_iter().map(tr_span).collect::<Vec<_>>()).style(style);
    out.alignment = alignment;
    out
}

pub fn tr_lines(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines.into_iter().map(tr_line).collect()
}

/// Ctrl+L: переключает английский и русский, запоминает выбор. Следующий кадр уже на новом языке.
fn toggle_language() {
    let next = if i18n::current() == Lang::Ru { i18n::Choice::En } else { i18n::Choice::Ru };
    i18n::set(next.resolve());
    let _ = i18n::save(next);
}

pub struct Tui {
    term: Terminal<CrosstermBackend<Stdout>>,
    started: Instant,
    /// Клавиши из `DISCORDCTL_TUI_KEYS` (для автоматических проверок); пока они есть, настоящий ввод не читается.
    script: VecDeque<KeyEvent>,
}

/// `enter down down space esc a …` — имена клавиш через пробел.
fn parse_script(s: &str) -> VecDeque<KeyEvent> {
    s.split_whitespace()
        .filter_map(|w| {
            let code = match w {
                "enter" => KeyCode::Enter,
                "esc" => KeyCode::Esc,
                "up" => KeyCode::Up,
                "down" => KeyCode::Down,
                "left" => KeyCode::Left,
                "right" => KeyCode::Right,
                "tab" => KeyCode::Tab,
                "space" => KeyCode::Char(' '),
                w if w.chars().count() == 1 => KeyCode::Char(w.chars().next()?),
                _ => return None,
            };
            Some(KeyEvent::new(code, KeyModifiers::NONE))
        })
        .collect()
}

impl Tui {
    pub fn new() -> Result<Tui> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        // Паника не должна оставить консоль в «сыром» режиме.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
        let mut term = Terminal::new(CrosstermBackend::new(stdout()))?;
        term.hide_cursor()?;
        term.clear()?;
        let script = std::env::var("DISCORDCTL_TUI_KEYS").map(|s| parse_script(&s)).unwrap_or_default();
        Ok(Tui { term, started: Instant::now(), script })
    }

    fn tick(&self) -> usize {
        (self.started.elapsed().as_millis() / 80) as usize
    }

    /// Рисует каркас и тело. `step` — номер подсвеченного шага, `hints` — подсказки по клавишам.
    pub fn draw(
        &mut self,
        step: usize,
        title: &str,
        hints: &[(&str, &str)],
        body: impl FnOnce(&mut Frame, Rect, usize),
    ) -> Result<()> {
        let tick = self.tick();
        self.term.draw(|f| {
            let area = shell(f, step, title, hints);
            body(f, area, tick);
        })?;
        Ok(())
    }

    /// Крутит экран, пока обработчик не вернёт результат. Ctrl+C даёт `Cancelled`.
    pub fn run<S, R>(
        &mut self,
        st: &mut S,
        step: usize,
        title: &str,
        hints: &[(&str, &str)],
        body: impl Fn(&mut Frame, Rect, &S, usize),
        mut key: impl FnMut(&mut S, KeyEvent) -> Flow<R>,
    ) -> Result<R> {
        loop {
            self.draw(step, title, hints, |f, a, t| body(f, a, st, t))?;
            if let Some(k) = self.script.pop_front() {
                thread::sleep(Duration::from_millis(400)); // экран успевает отрисоваться
                if let Flow::Done(r) = key(st, k) {
                    return Ok(r);
                }
                continue;
            }
            if event::poll(Duration::from_millis(80))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                        return Err(anyhow!(Cancelled));
                    }
                    if k.code == KeyCode::Char('l') && k.modifiers.contains(KeyModifiers::CONTROL) {
                        toggle_language();
                        continue;
                    }
                    if let Flow::Done(r) = key(st, k) {
                        return Ok(r);
                    }
                }
            }
        }
    }

    /// Выполняет `f` в соседнем потоке и показывает ход работы. Сообщения `f` становятся строками списка;
    /// сообщение с `~` в начале только меняет текст рядом со спиннером.
    pub fn work<T: Send>(&mut self, step: usize, title: &str, f: impl FnOnce(&dyn Fn(&str)) -> T + Send) -> Result<T> {
        let (tx, rx) = mpsc::channel::<String>();
        thread::scope(|scope| {
            let handle = scope.spawn(move || f(&|m: &str| drop(tx.send(m.to_owned()))));
            let mut lines: Vec<String> = vec![];
            let mut status = String::new();
            loop {
                for m in rx.try_iter() {
                    match m.strip_prefix('~') {
                        Some(s) => status = s.to_owned(),
                        None => {
                            status.clear();
                            lines.push(m);
                        }
                    }
                }
                let running = !handle.is_finished();
                self.draw(step, title, &[("wait", "working…")], |f, area, tick| {
                    let rows: Vec<Line> = lines
                        .iter()
                        .enumerate()
                        .map(|(i, l)| {
                            if running && i + 1 == lines.len() {
                                let mut spans =
                                    vec![Span::styled(spinner(tick), bold(ACCENT)), Span::raw(format!(" {}", tr(l)))];
                                if !status.is_empty() {
                                    spans.push(Span::styled(format!("  {}", tr(&status)), dim()));
                                }
                                Line::from(spans)
                            } else {
                                Line::from(vec![Span::styled("√", bold(OK)), Span::raw(format!(" {}", tr(l)))])
                            }
                        })
                        .collect();
                    f.render_widget(Paragraph::new(rows), area);
                })?;
                if !running {
                    break;
                }
                if event::poll(Duration::from_millis(80))? {
                    let _ = event::read()?; // клавиши во время работы игнорируем
                }
            }
            handle.join().map_err(|_| anyhow!("a background task panicked"))
        })
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.term.show_cursor();
        restore();
    }
}

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);
}

/// Каркас: шапка, шаги, подсказки внизу. Возвращает область для содержимого.
fn shell(f: &mut Frame, step: usize, title: &str, hints: &[(&str, &str)]) -> Rect {
    let full = f.area();
    let w = full.width.saturating_sub(4).min(MAX_WIDTH);
    let area = Rect { x: full.x + (full.width.saturating_sub(w)) / 2, width: w, y: full.y, height: full.height };
    let [head, stepper, rule, body, foot] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);

    let logo = Line::from(vec![
        Span::styled("◆ ", bold(ACCENT)),
        Span::styled("discordctl", bold(Color::White)),
        Span::styled(format!("  {}", tr("your Discord, from the terminal and your AI agents")), dim()),
    ]);
    f.render_widget(Paragraph::new(vec![Line::raw(""), logo]), head);
    f.render_widget(
        Paragraph::new(Span::styled(format!("v{} ", env!("CARGO_PKG_VERSION")), dim())).alignment(Alignment::Right),
        head,
    );

    let mut spans = vec![];
    let steps = if i18n::current() == Lang::Ru { &STEPS_RU } else { &STEPS };
    for (i, name) in steps.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ──  ", fg(Color::Rgb(70, 74, 100))));
        }
        spans.push(match i.cmp(&step) {
            std::cmp::Ordering::Less => Span::styled(format!("√ {name}"), fg(OK)),
            std::cmp::Ordering::Equal => Span::styled(format!("● {name}"), bold(ACCENT)),
            std::cmp::Ordering::Greater => Span::styled(format!("○ {name}"), dim()),
        });
    }
    f.render_widget(Paragraph::new(Line::from(spans)), stepper);
    f.render_widget(Paragraph::new(Span::styled("─".repeat(area.width as usize), fg(Color::Rgb(55, 58, 82)))), rule);

    let block = Block::new().padding(ratatui::widgets::Padding::new(0, 0, 1, 0));
    let inner = block.inner(body);
    f.render_widget(block, body);
    // Заголовок экрана.
    let [title_area, content] = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
    f.render_widget(Paragraph::new(Span::styled(tr(title).into_owned(), bold(Color::White))), title_area);

    let mut hint_spans = vec![];
    for (k, what) in hints {
        if !hint_spans.is_empty() {
            hint_spans.push(Span::styled("   ", dim()));
        }
        hint_spans.push(Span::styled(tr(k).into_owned(), bold(ACCENT)));
        hint_spans.push(Span::styled(format!(" {}", tr(what)), dim()));
    }
    // Смена языка доступна на любом экране, кроме «работаю…», где клавиши не читаются.
    if !hints.iter().any(|(k, _)| *k == "wait") {
        hint_spans.push(Span::styled("   ", dim()));
        hint_spans.push(Span::styled("Ctrl+L", bold(ACCENT)));
        hint_spans.push(Span::styled(if i18n::current() == Lang::Ru { " English" } else { " Русский" }, dim()));
    }
    f.render_widget(Paragraph::new(Line::from(hint_spans)), foot);
    content
}

// ---- виджеты -------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Флажок `[√]`, переключается пробелом.
    Toggle,
    /// Переключатель `(●)`: выбран ровно один.
    Radio,
    /// Кнопка действия внизу списка.
    Action,
}

#[derive(Clone)]
pub struct Item {
    pub kind: Kind,
    pub title: String,
    pub desc: String,
    pub on: bool,
    /// Нельзя изменить (показывается с пометкой).
    pub locked: bool,
    pub tag: Option<(String, Color)>,
}

impl Item {
    pub fn toggle(title: impl Into<String>, desc: impl Into<String>, on: bool) -> Item {
        Item { kind: Kind::Toggle, title: title.into(), desc: desc.into(), on, locked: false, tag: None }
    }
    pub fn radio(title: impl Into<String>, desc: impl Into<String>, on: bool) -> Item {
        Item { kind: Kind::Radio, ..Item::toggle(title, desc, on) }
    }
    pub fn action(title: impl Into<String>, desc: impl Into<String>) -> Item {
        Item { kind: Kind::Action, ..Item::toggle(title, desc, false) }
    }
    pub fn tag(mut self, text: impl Into<String>, color: Color) -> Item {
        self.tag = Some((text.into(), color));
        self
    }
    pub fn locked(mut self) -> Item {
        self.locked = true;
        self
    }
}

/// Список выбора с курсором. Стрелки двигают, пробел переключает, Enter подтверждает действие.
pub struct Choice {
    pub items: Vec<Item>,
    pub cursor: usize,
}

impl Choice {
    pub fn new(items: Vec<Item>) -> Choice {
        // Курсор сразу на первой кнопке, если список — только кнопки; иначе на первом пункте.
        Choice { items, cursor: 0 }
    }

    pub fn up(&mut self) {
        self.cursor = if self.cursor == 0 { self.items.len() - 1 } else { self.cursor - 1 };
    }
    pub fn down(&mut self) {
        self.cursor = (self.cursor + 1) % self.items.len();
    }

    /// Пробел или Enter на пункте: переключает флажок или выбирает переключатель.
    pub fn toggle(&mut self) {
        let kind = self.items[self.cursor].kind;
        if self.items[self.cursor].locked {
            return;
        }
        match kind {
            Kind::Toggle => self.items[self.cursor].on ^= true,
            Kind::Radio => {
                for (i, it) in self.items.iter_mut().enumerate() {
                    if it.kind == Kind::Radio {
                        it.on = i == self.cursor;
                    }
                }
            }
            Kind::Action => {}
        }
    }

    pub fn on_action(&self) -> bool {
        self.items[self.cursor].kind == Kind::Action
    }

    /// Общая обработка клавиш; `true`, если клавиша была навигацией.
    pub fn key(&mut self, k: KeyEvent) -> bool {
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.up(),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => self.down(),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.items.len() - 1,
            KeyCode::Char(' ') => self.toggle(),
            _ => return false,
        }
        true
    }

    pub fn lines(&self) -> Vec<Line<'static>> {
        let mut out = vec![];
        for (i, it) in self.items.iter().enumerate() {
            let here = i == self.cursor;
            let row_bg = if here { Style::new().bg(ROW_BG) } else { Style::new() };
            let pointer = if here { Span::styled("❯ ", bold(ACCENT)) } else { Span::raw("  ") };
            let mark = match it.kind {
                Kind::Toggle if it.on => Span::styled("[√] ", bold(OK)),
                Kind::Toggle => Span::styled("[ ] ", dim()),
                Kind::Radio if it.on => Span::styled("(●) ", bold(ACCENT)),
                Kind::Radio => Span::styled("( ) ", dim()),
                Kind::Action => Span::raw(""),
            };
            let title_style = match (it.kind, here) {
                (Kind::Action, true) => Style::new().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD),
                (Kind::Action, false) => bold(ACCENT),
                (_, true) => bold(Color::White),
                (_, false) => Style::new(),
            };
            let title = if it.kind == Kind::Action { format!(" {} ", it.title) } else { it.title.clone() };
            let mut spans = vec![pointer, mark, Span::styled(title, title_style)];
            if let Some((t, c)) = &it.tag {
                spans.push(Span::styled(format!("  {t}"), fg(*c)));
            }
            if it.locked {
                spans.push(Span::styled("  (required)", dim()));
            }
            out.push(Line::from(spans).style(if it.kind == Kind::Action { Style::new() } else { row_bg }));
            if !it.desc.is_empty() {
                out.push(Line::from(vec![
                    Span::raw(if it.kind == Kind::Action { "    " } else { "      " }),
                    Span::styled(it.desc.clone(), dim()),
                ]));
            }
            if it.kind != Kind::Action || i + 1 < self.items.len() {
                out.push(Line::raw(""));
            }
        }
        out
    }
}

/// Рамка со скруглёнными углами и заголовком; возвращает внутреннюю область.
pub fn panel(f: &mut Frame, area: Rect, title: &str, color: Color) -> Rect {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(color))
        .title(Span::styled(format!(" {} ", tr(title)), bold(color)))
        .padding(ratatui::widgets::Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

pub fn text(f: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    f.render_widget(Paragraph::new(tr_lines(lines)).wrap(Wrap { trim: false }), area);
}

/// Сколько строк займёт текст при данной ширине (для расчёта высоты рамок).
pub fn height_of(lines: &[Line], width: u16) -> u16 {
    let w = width.max(1) as usize;
    lines.iter().map(|l| (l.width().max(1)).div_ceil(w)).sum::<usize>() as u16
}

pub fn first_upper(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}
