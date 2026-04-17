use crate::history::StringHistoryList;
use crate::output::format_duration;
use crate::repository::{QueryOpts, Repository};
use crate::service::{SessionService, SessionStatus, summarize};
use anyhow::Result;
use chrono::{DateTime, Local, Months, NaiveTime, TimeDelta, Utc};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use ratatui_textarea::{CursorMove, TextArea};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Timespan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Timespan {
    All,
    Day1,
    Day3,
    Day7,
    Month1,
    Month3,
    Year1,
}

impl Timespan {
    const PRESETS: &'static [Self] = &[
        Self::All,
        Self::Day1,
        Self::Day3,
        Self::Day7,
        Self::Month1,
        Self::Month3,
        Self::Year1,
    ];

    fn next(self) -> Self {
        let idx = Self::PRESETS.iter().position(|&p| p == self).unwrap_or(0);
        Self::PRESETS[(idx + 1) % Self::PRESETS.len()]
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Day1 => "1d",
            Self::Day3 => "3d",
            Self::Day7 => "7d",
            Self::Month1 => "1M",
            Self::Month3 => "3M",
            Self::Year1 => "1y",
        }
    }

    /// Lower bound for this span relative to `now`. `None` means all time.
    fn lower_bound(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::All => None,
            Self::Day1 => Some(now - TimeDelta::days(1)),
            Self::Day3 => Some(now - TimeDelta::days(3)),
            Self::Day7 => Some(now - TimeDelta::days(7)),
            Self::Month1 => now
                .date_naive()
                .checked_sub_months(Months::new(1))
                .map(|d| d.and_time(now.time()).and_utc()),
            Self::Month3 => now
                .date_naive()
                .checked_sub_months(Months::new(3))
                .map(|d| d.and_time(now.time()).and_utc()),
            Self::Year1 => now
                .date_naive()
                .checked_sub_months(Months::new(12))
                .map(|d| d.and_time(now.time()).and_utc()),
        }
    }
}

// ---------------------------------------------------------------------------
// Prompt state
// ---------------------------------------------------------------------------

/// State carried while the description prompt is open.
struct PromptState {
    textarea: TextArea<'static>,
    /// The user's own typed text, captured the moment Up is first pressed.
    saved_input: Arc<str>,
    /// Index into desc_history while navigating; `None` means the user's own
    /// input is shown.
    history_idx: Option<usize>,
    /// Block title shown on the textarea border.
    title: &'static str,
}

enum Mode {
    Normal,
    Prompting(Box<PromptState>),
    Renaming(Box<PromptState>),
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct App<R: Repository> {
    service: SessionService<R>,
    mode: Mode,
    /// UI-level summary span preset; translated to a datetime before querying.
    summary_span: Timespan,
    /// Lower bound of the summary window; `None` means all time.
    summary_from: Option<DateTime<Utc>>,
    /// MRU task history for the prompt's up/down autocomplete.
    desc_history: StringHistoryList,
    /// Stable summary of completed sessions for the current window.
    /// Rebuilt on every state transition; never inside the render loop.
    summary_cache: Vec<(Arc<str>, TimeDelta)>,
    /// Total completed-session time since today's UTC midnight.
    base_duration_today: TimeDelta,
    exit: bool,
}

impl<R: Repository> App<R> {
    fn new(service: SessionService<R>) -> Result<Self> {
        let today = today_utc_midnight();

        let (desc_history, base_duration_today, summary_cache) = {
            let entries = service.list(QueryOpts::default())?;

            let mut desc_history = StringHistoryList::new();
            for e in &entries {
                desc_history.push_str(&e.desc);
            }

            let base_duration_today: TimeDelta = entries
                .iter()
                .filter(|e| e.interval.start >= today && e.interval.end.is_some())
                .map(|e| e.interval.duration())
                .sum();

            let summary_cache = summarize(entries.as_slice());

            (desc_history, base_duration_today, summary_cache)
        };

        Ok(App {
            service,
            mode: Mode::Normal,
            summary_span: Timespan::All,
            summary_from: None,
            desc_history,
            summary_cache,
            base_duration_today,
            exit: false,
        })
    }

    fn open_prompt(&self) -> Box<PromptState> {
        Box::new(PromptState {
            textarea: Self::make_textarea("", " new session "),
            saved_input: "".into(),
            history_idx: None,
            title: " new session ",
        })
    }

    fn open_rename_prompt(&self) -> Option<Box<PromptState>> {
        let current_desc = match self.service.status() {
            SessionStatus::Active { desc, .. } | SessionStatus::Paused { desc } => desc.clone(),
            SessionStatus::Idle => return None,
        };
        Some(Box::new(PromptState {
            textarea: Self::make_textarea(&current_desc, " rename session "),
            saved_input: current_desc,
            history_idx: None,
            title: " rename session ",
        }))
    }

    /// Create a styled textarea pre-filled with `content`, cursor at end-of-line.
    fn make_textarea(content: &str, title: &'static str) -> TextArea<'static> {
        let mut ta = TextArea::from([content.to_owned()]);
        ta.set_placeholder_text("task description...");
        ta.set_cursor_line_style(Style::default());
        ta.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(title),
        );
        ta.move_cursor(CursorMove::End);
        ta
    }

    // -----------------------------------------------------------------------
    // Session operations
    // -----------------------------------------------------------------------

    fn start_session(&mut self, desc: Arc<str>, now: DateTime<Utc>) -> Result<()> {
        // Capture previous session's start time before mutating.
        let prev_started_at = match self.service.status() {
            SessionStatus::Active { started_at, .. } => Some(*started_at),
            _ => None,
        };

        self.service.start(desc.clone(), now)?;

        if let Some(started_at) = prev_started_at {
            // start_session auto-closes the previous session at now - 1s.
            self.accrue(started_at, now - TimeDelta::seconds(1), now);
        }
        self.desc_history.push_str(&desc);
        self.rebuild_summary_cache();
        Ok(())
    }

    fn toggle_pause(&mut self) -> Result<()> {
        let now = Utc::now();
        match self.service.status() {
            SessionStatus::Active { started_at, .. } => {
                let started_at = *started_at;
                self.service.pause(now)?;
                self.accrue(started_at, now, now);
                self.rebuild_summary_cache();
                Ok(())
            }
            SessionStatus::Paused { .. } => self.service.resume(now),
            SessionStatus::Idle => Ok(()),
        }
    }

    fn end_or_quit(&mut self) -> Result<()> {
        let now = Utc::now();
        match self.service.status() {
            SessionStatus::Active { started_at, .. } => {
                let started_at = *started_at;
                self.service.end(now)?;
                self.accrue(started_at, now, now);
                self.rebuild_summary_cache();
                Ok(())
            }
            SessionStatus::Paused { .. } => {
                self.service.discard_paused();
                Ok(())
            }
            SessionStatus::Idle => {
                self.exit = true;
                Ok(())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Aggregation helpers
    // -----------------------------------------------------------------------

    fn summary_for_display(&self, now: DateTime<Utc>) -> Vec<(Arc<str>, TimeDelta)> {
        let SessionStatus::Active { desc, started_at } = self.service.status() else {
            return self.summary_cache.clone();
        };

        let effective_start = (*started_at).max(self.summary_from.unwrap_or(*started_at));
        let running = (now - effective_start).max(TimeDelta::zero());

        let mut result = self.summary_cache.clone();
        match result.iter_mut().find(|(d, _)| d == desc) {
            Some(row) => row.1 += running,
            None => result.push((desc.clone(), running)),
        }
        result
    }

    fn duration_today(&self, now: DateTime<Utc>) -> TimeDelta {
        let since_24h = now - TimeDelta::hours(24);
        let running = match self.service.status() {
            SessionStatus::Active { started_at, .. } if *started_at >= since_24h => {
                (now - *started_at).max(TimeDelta::zero())
            }
            _ => TimeDelta::zero(),
        };
        self.base_duration_today + running
    }

    fn set_summary_window(&mut self, from: Option<DateTime<Utc>>) {
        self.summary_from = from;
        self.rebuild_summary_cache();
    }

    fn rebuild_summary_cache(&mut self) {
        let summary = {
            let entries = self
                .service
                .list(QueryOpts {
                    from: self.summary_from,
                    ..Default::default()
                })
                .unwrap_or_default();
            summarize(&entries)
        };
        self.summary_cache = summary;
    }

    /// Accrue a just-completed interval into `base_duration_today` if it falls
    /// within the last 24 hours.
    fn accrue(&mut self, started_at: DateTime<Utc>, ended_at: DateTime<Utc>, now: DateTime<Utc>) {
        let since_24h = now - TimeDelta::hours(24);
        if started_at >= since_24h {
            let elapsed = (ended_at - started_at).max(TimeDelta::zero());
            self.base_duration_today += elapsed;
        }
    }

    // -----------------------------------------------------------------------
    // Event loop
    // -----------------------------------------------------------------------

    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.exit {
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(250))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.handle_key(key)?;
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        match mode {
            Mode::Prompting(mut p) => {
                match key.code {
                    KeyCode::Esc => { /* mode stays Normal – cancels the prompt */ }

                    KeyCode::Enter => {
                        let desc = p
                            .textarea
                            .lines()
                            .first()
                            .map(|l| l.trim().to_lowercase())
                            .unwrap_or_default();
                        if !desc.is_empty() {
                            self.start_session(desc.into(), Utc::now())?;
                            // mode stays Normal
                        } else {
                            self.mode = Mode::Prompting(p);
                        }
                    }

                    KeyCode::Up => {
                        let (next_idx, content) = {
                            let history = &self.desc_history;
                            let next_idx = match p.history_idx {
                                None if !history.is_empty() => {
                                    p.saved_input = p
                                        .textarea
                                        .lines()
                                        .first()
                                        .cloned()
                                        .unwrap_or_default()
                                        .into();
                                    Some(0)
                                }
                                Some(i) if i + 1 < history.len() => Some(i + 1),
                                other => other,
                            };
                            let content = next_idx.map(|i| history[i].to_owned());
                            (next_idx, content)
                        };
                        if next_idx != p.history_idx {
                            p.history_idx = next_idx;
                            p.textarea = Self::make_textarea(&content.unwrap_or_default(), p.title);
                        }
                        self.mode = Mode::Prompting(p);
                    }

                    KeyCode::Down => {
                        match p.history_idx {
                            Some(0) => {
                                p.history_idx = None;
                                let saved = p.saved_input.clone();
                                p.textarea = Self::make_textarea(&saved, p.title);
                            }
                            Some(i) => {
                                let content = self.desc_history[i - 1].to_owned();
                                p.history_idx = Some(i - 1);
                                p.textarea = Self::make_textarea(&content, p.title);
                            }
                            None => {}
                        }
                        self.mode = Mode::Prompting(p);
                    }

                    _ => {
                        p.history_idx = None;
                        p.textarea.input(key);
                        self.mode = Mode::Prompting(p);
                    }
                }
            }

            Mode::Renaming(mut p) => {
                match key.code {
                    KeyCode::Esc => { /* mode stays Normal – cancels the rename */ }

                    KeyCode::Enter => {
                        let desc = p
                            .textarea
                            .lines()
                            .first()
                            .map(|l| l.trim().to_lowercase())
                            .unwrap_or_default();
                        if !desc.is_empty() {
                            self.rename_session(&desc)?;
                            // mode stays Normal
                        } else {
                            self.mode = Mode::Renaming(p);
                        }
                    }

                    KeyCode::Up => {
                        let (next_idx, content) = {
                            let history = &self.desc_history;
                            let next_idx = match p.history_idx {
                                None if !history.is_empty() => {
                                    p.saved_input = p
                                        .textarea
                                        .lines()
                                        .first()
                                        .cloned()
                                        .unwrap_or_default()
                                        .into();
                                    Some(0)
                                }
                                Some(i) if i + 1 < history.len() => Some(i + 1),
                                other => other,
                            };
                            let content = next_idx.map(|i| history[i].to_owned());
                            (next_idx, content)
                        };
                        if next_idx != p.history_idx {
                            p.history_idx = next_idx;
                            p.textarea = Self::make_textarea(&content.unwrap_or_default(), p.title);
                        }
                        self.mode = Mode::Renaming(p);
                    }

                    KeyCode::Down => {
                        match p.history_idx {
                            Some(0) => {
                                p.history_idx = None;
                                let saved = p.saved_input.clone();
                                p.textarea = Self::make_textarea(&saved, p.title);
                            }
                            Some(i) => {
                                let content = self.desc_history[i - 1].to_owned();
                                p.history_idx = Some(i - 1);
                                p.textarea = Self::make_textarea(&content, p.title);
                            }
                            None => {}
                        }
                        self.mode = Mode::Renaming(p);
                    }

                    _ => {
                        p.history_idx = None;
                        p.textarea.input(key);
                        self.mode = Mode::Renaming(p);
                    }
                }
            }

            Mode::Normal => match key.code {
                KeyCode::Char('q') => self.exit = true,
                KeyCode::Char('r') => {
                    if let Some(p) = self.open_rename_prompt() {
                        self.mode = Mode::Renaming(p);
                    }
                }
                KeyCode::Char('s') => {
                    self.summary_span = self.summary_span.next();
                    let from = self.summary_span.lower_bound(Utc::now());
                    self.set_summary_window(from);
                }
                KeyCode::Char(' ') => self.toggle_pause()?,
                KeyCode::Esc => self.end_or_quit()?,
                KeyCode::Enter => self.mode = Mode::Prompting(self.open_prompt()),
                _ => {}
            },
        }
        Ok(())
    }

    fn rename_session(&mut self, new_desc: &str) -> Result<()> {
        self.service.rename(new_desc.into())?;
        self.desc_history.push_str(new_desc);
        self.rebuild_summary_cache();
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let now = Utc::now();
        let clock = now
            .with_timezone(&Local)
            .format("%Y-%m-%d  %H:%M:%S")
            .to_string();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(" hourus ").centered())
            .title_top(Line::from(format!(" {clock} ")).right_aligned())
            .border_style(Style::default().fg(Color::DarkGray));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let [content, hints] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

        match &self.mode {
            Mode::Normal => self.draw_normal(frame, content, now),
            Mode::Prompting(p) | Mode::Renaming(p) => self.draw_prompting(frame, content, p),
        }
        self.draw_hints(frame, hints);
    }

    fn draw_normal(&self, frame: &mut Frame, area: Rect, now: DateTime<Utc>) {
        let [status_area, summary_area] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(area);

        // --- Left: session status ---
        let mut lines: Vec<Line> = vec![Line::from("")];

        match self.service.status() {
            SessionStatus::Active { desc, started_at } => {
                let elapsed = (now - *started_at).max(TimeDelta::zero());
                let local_start = started_at.with_timezone(&Local);
                lines.push(Line::from(vec![
                    Span::styled("  ● ", Style::default().fg(Color::Green)),
                    Span::styled(desc.as_ref(), Style::default().add_modifier(Modifier::BOLD)),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("    started "),
                    Span::styled(
                        local_start.format("%H:%M:%S").to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw("  ·  running "),
                    Span::styled(format_duration(elapsed), Style::default().fg(Color::Green)),
                ]));
            }
            SessionStatus::Paused { desc } => {
                lines.push(Line::from(vec![
                    Span::styled("  ⏸ ", Style::default().fg(Color::Yellow)),
                    Span::styled(desc.as_ref(), Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled("  (paused)", Style::default().fg(Color::DarkGray)),
                ]));
                lines.push(Line::from(Span::styled(
                    "    press space to resume",
                    Style::default().fg(Color::DarkGray),
                )));
            }
            SessionStatus::Idle => {
                lines.push(Line::from(Span::styled(
                    "  No active session.",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(Span::styled(
                    "  Press enter to start one.",
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  logged today  "),
            Span::styled(
                format_duration(self.duration_today(now)),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        frame.render_widget(Paragraph::new(lines), status_area);

        let summary = self.summary_for_display(now);
        self.draw_summary(frame, summary_area, &summary);
    }

    fn draw_summary(&self, frame: &mut Frame, area: Rect, summary: &[(Arc<str>, TimeDelta)]) {
        let mut lines: Vec<Line> = vec![];

        lines.push(Line::from(vec![
            Span::styled("  tasks ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("[{}]", self.summary_span.label()),
                Style::default().fg(Color::Cyan),
            ),
        ]));

        if summary.is_empty() {
            lines.push(Line::from(Span::styled(
                "  no entries",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            let total: TimeDelta = summary.iter().map(|(_, d)| *d).sum();
            let max_name = summary.iter().map(|(s, _)| s.len()).max().unwrap_or(0);
            let col_width = max_name + 2;

            for (desc, dur) in summary {
                let pad = col_width - desc.len();
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::raw(desc.as_ref()),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(format_duration(*dur), Style::default().fg(Color::Cyan)),
                ]));
            }

            let total_label = "total";
            let pad = col_width.saturating_sub(total_label.len());
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(total_label, Style::default().fg(Color::DarkGray)),
                Span::raw(" ".repeat(pad)),
                Span::styled(
                    format_duration(total),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        frame.render_widget(Paragraph::new(lines), area);
    }

    fn draw_prompting(&self, frame: &mut Frame, area: Rect, p: &PromptState) {
        let [_, ta_area, _] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .areas(area);

        let [_, ta_inner, _] = Layout::horizontal([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .areas(ta_area);

        frame.render_widget(&p.textarea, ta_inner);
    }

    fn draw_hints(&self, frame: &mut Frame, area: Rect) {
        let style_key = Style::default().bg(Color::DarkGray).fg(Color::White);
        let style_dim = Style::default().fg(Color::DarkGray);

        let sep = || Span::styled("  ", style_dim);
        let key = |label: &'static str| Span::styled(format!(" {label} "), style_key);
        let txt = |label: &'static str| Span::raw(format!(" {label}"));

        let hints: Line = match &self.mode {
            Mode::Prompting(_) => Line::from(vec![
                key("↑↓"),
                txt("history"),
                sep(),
                key("enter"),
                txt("confirm"),
                sep(),
                key("esc"),
                txt("cancel"),
            ]),
            Mode::Renaming(_) => Line::from(vec![
                key("↑↓"),
                txt("history"),
                sep(),
                key("enter"),
                txt("rename"),
                sep(),
                key("esc"),
                txt("cancel"),
            ]),
            Mode::Normal => match self.service.status() {
                SessionStatus::Active { .. } => Line::from(vec![
                    key("space"),
                    txt("pause"),
                    sep(),
                    key("esc"),
                    txt("end"),
                    sep(),
                    key("enter"),
                    txt("new task"),
                    sep(),
                    key("r"),
                    txt("rename"),
                    sep(),
                    key("s"),
                    txt("span"),
                    sep(),
                    key("q"),
                    txt("quit"),
                ]),
                SessionStatus::Paused { .. } => Line::from(vec![
                    key("space"),
                    txt("resume"),
                    sep(),
                    key("esc"),
                    txt("discard"),
                    sep(),
                    key("enter"),
                    txt("new task"),
                    sep(),
                    key("r"),
                    txt("rename"),
                    sep(),
                    key("s"),
                    txt("span"),
                    sep(),
                    key("q"),
                    txt("quit"),
                ]),
                SessionStatus::Idle => Line::from(vec![
                    key("enter"),
                    txt("start"),
                    sep(),
                    key("s"),
                    txt("span"),
                    sep(),
                    key("esc"),
                    txt("quit"),
                    sep(),
                    key("q"),
                    txt("quit"),
                ]),
            },
        };

        frame.render_widget(Paragraph::new(hints), area);
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run<R: Repository>(
    mut service: SessionService<R>,
    initial_desc: Option<String>,
) -> Result<()> {
    if let Some(desc) = initial_desc {
        service.start(desc.into(), Utc::now())?;
    }
    let mut app = App::new(service)?;
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    app.service.flush()?;
    result
}

fn today_utc_midnight() -> DateTime<Utc> {
    Utc::now().date_naive().and_time(NaiveTime::MIN).and_utc()
}
