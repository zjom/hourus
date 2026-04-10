use crate::entry::{EntryKind, EntryLine};
use crate::output::format_duration;
use crate::report::Report;
use crate::storage::{FileStorage, Storage};
use anyhow::Result;
use chrono::{Local, NaiveDateTime, TimeDelta};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use ratatui_textarea::{CursorMove, TextArea};
use std::collections::{HashSet, VecDeque};
use std::time::Duration;

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

enum SessionState {
    Idle,
    Active {
        desc: String,
        started_at: NaiveDateTime,
    },
    Paused {
        desc: String,
    },
}

/// State carried while the description prompt is open.
struct PromptState {
    textarea: TextArea<'static>,
    /// The user's own typed text, captured the moment Up is first pressed.
    saved_input: String,
    /// Index into `App::history` while navigating; `None` means the user's own input.
    history_idx: Option<usize>,
}

enum Mode {
    Normal,
    Prompting(PromptState),
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct App {
    session: SessionState,
    mode: Mode,
    storage: FileStorage,
    /// Unique task descriptions, most-recent-first. Built from START lines, deduped.
    history: VecDeque<String>,
    history_set: HashSet<String>,
    /// Completed-session time in the past 24 h, updated whenever a session ends/pauses.
    base_duration_today: TimeDelta,
    exit: bool,
}

impl App {
    fn new(storage: FileStorage) -> Result<Self> {
        let lines = storage.load()?;

        let state_report = Report::new().with_lines(lines.clone()).build()?;

        // Determine current session state from the last entry line.
        let session = match state_report.entry_lines.last() {
            Some(last) if last.kind == EntryKind::Start => SessionState::Active {
                desc: last.desc.clone(),
                started_at: last.dt,
            },
            _ => SessionState::Idle,
        };

        // Build deduped history from all START lines, most-recent-first.
        let mut history_set = HashSet::new();
        let history = Self::build_history(&mut history_set, &state_report.entry_lines);

        // Pre-compute completed session time for the past 24 h.
        let since_today = Local::now()
            .naive_local()
            .date()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let base_duration_today = Report::new()
            .with_lines(lines)
            .from(since_today)
            .build()?
            .total_duration();

        Ok(App {
            session,
            mode: Mode::Normal,
            storage,
            history,
            history_set,
            base_duration_today,
            exit: false,
        })
    }

    /// Build a deduplicated list of task descriptions (most-recent-first) from entry lines.
    fn build_history(seen: &mut HashSet<String>, entry_lines: &[EntryLine]) -> VecDeque<String> {
        entry_lines
            .iter()
            .rev()
            .filter(|l| l.kind == EntryKind::Start)
            .filter_map(|l| seen.insert(l.desc.clone()).then(|| l.desc.clone()))
            .collect()
    }

    /// Prepend `desc` to history, removing any prior occurrence so there are no duplicates.
    fn push_history(&mut self, desc: &str) {
        if !self.history_set.insert(desc.to_owned()) {
            self.history.retain(|d| d != desc);
        }
        self.history.push_front(desc.to_owned());
    }

    /// Total logged in the past 24 h, including the currently running session.
    fn duration_today(&self) -> TimeDelta {
        let since_24h = Local::now().naive_local() - TimeDelta::hours(24);
        let running = match &self.session {
            SessionState::Active { started_at, .. } if *started_at >= since_24h => {
                (Local::now().naive_local() - *started_at).max(TimeDelta::zero())
            }
            _ => TimeDelta::zero(),
        };
        self.base_duration_today + running
    }

    /// Create a styled textarea, pre-filled with `content` and cursor at end-of-line.
    fn make_textarea(content: &str) -> TextArea<'static> {
        let mut ta = TextArea::from([content.to_owned()]);
        ta.set_placeholder_text("task description...");
        ta.set_cursor_line_style(Style::default());
        ta.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" new session "),
        );
        ta.move_cursor(CursorMove::End);
        ta
    }

    fn open_prompt(&self) -> PromptState {
        PromptState {
            textarea: Self::make_textarea(""),
            saved_input: String::new(),
            history_idx: None,
        }
    }

    fn append_line(&mut self, kind: EntryKind, dt: NaiveDateTime, desc: &str) -> Result<()> {
        Ok(self.storage.append(&[EntryLine {
            kind,
            dt,
            desc: desc.to_owned(),
        }])?)
    }

    /// Account for a just-completed session interval in `base_duration_today`.
    fn accrue(&mut self, started_at: NaiveDateTime, ended_at: NaiveDateTime) {
        let since_24h = ended_at - TimeDelta::hours(24);
        if started_at >= since_24h {
            let elapsed = (ended_at - started_at).max(TimeDelta::zero());
            self.base_duration_today = self.base_duration_today + elapsed;
        }
    }

    // -----------------------------------------------------------------------
    // Session operations
    // -----------------------------------------------------------------------

    /// Space: pause a running session, or resume a paused one.
    fn toggle_pause(&mut self) -> Result<()> {
        let now = Local::now().naive_local();
        let prev = std::mem::replace(&mut self.session, SessionState::Idle);
        match prev {
            SessionState::Active { desc, started_at } => {
                self.append_line(EntryKind::End, now, &desc)?;
                self.accrue(started_at, now);
                self.session = SessionState::Paused { desc };
            }
            SessionState::Paused { desc } => {
                self.append_line(EntryKind::Start, now, &desc)?;
                self.session = SessionState::Active {
                    desc,
                    started_at: now,
                };
            }
            idle => self.session = idle,
        }
        Ok(())
    }

    /// Esc: end the active session, discard a paused one, or quit if already idle.
    fn end_or_quit(&mut self) -> Result<()> {
        let now = Local::now().naive_local();
        let prev = std::mem::replace(&mut self.session, SessionState::Idle);
        match prev {
            SessionState::Active { desc, started_at } => {
                self.append_line(EntryKind::End, now, &desc)?;
                self.accrue(started_at, now);
            }
            SessionState::Paused { .. } => { /* discard, session stays Idle */ }
            SessionState::Idle => self.exit = true,
        }
        Ok(())
    }

    /// Start a new session with `desc`, ending the current one first if necessary.
    fn start_session(&mut self, desc: &str, now: NaiveDateTime) -> Result<()> {
        let prev = std::mem::replace(&mut self.session, SessionState::Idle);
        let start_dt = match prev {
            SessionState::Active {
                desc: old_desc,
                started_at,
            } => {
                self.append_line(EntryKind::End, now, &old_desc)?;
                self.accrue(started_at, now);
                now + TimeDelta::seconds(1)
            }
            _ => now,
        };
        self.append_line(EntryKind::Start, start_dt, desc)?;
        self.push_history(desc);
        self.session = SessionState::Active {
            desc: desc.to_owned(),
            started_at: start_dt,
        };
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Event loop
    // -----------------------------------------------------------------------

    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.exit {
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(250))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        // Move mode out to get an owned value, avoiding borrow conflicts.
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
                            self.start_session(&desc, Local::now().naive_local())?;
                            // mode stays Normal
                        } else {
                            self.mode = Mode::Prompting(p);
                        }
                    }

                    KeyCode::Up => {
                        let next_idx = match p.history_idx {
                            None if !self.history.is_empty() => {
                                // Capture the user's current typed text before navigating.
                                p.saved_input =
                                    p.textarea.lines().first().cloned().unwrap_or_default();
                                Some(0)
                            }
                            Some(i) if i + 1 < self.history.len() => Some(i + 1),
                            other => other, // already at oldest, or history empty
                        };
                        if next_idx != p.history_idx {
                            p.history_idx = next_idx;
                            p.textarea = Self::make_textarea(&self.history[next_idx.unwrap()]);
                        }
                        self.mode = Mode::Prompting(p);
                    }

                    KeyCode::Down => {
                        match p.history_idx {
                            Some(0) => {
                                p.history_idx = None;
                                let saved = p.saved_input.clone();
                                p.textarea = Self::make_textarea(&saved);
                            }
                            Some(i) => {
                                p.history_idx = Some(i - 1);
                                p.textarea = Self::make_textarea(&self.history[i - 1]);
                            }
                            None => {} // already at user's own input
                        }
                        self.mode = Mode::Prompting(p);
                    }

                    _ => {
                        // Any other key: if navigating history, exit navigation mode
                        // (the textarea keeps whatever was shown – the user is now editing it).
                        p.history_idx = None;
                        p.textarea.input(key);
                        self.mode = Mode::Prompting(p);
                    }
                }
            }

            Mode::Normal => match key.code {
                KeyCode::Char('q') => self.exit = true,
                KeyCode::Char(' ') => self.toggle_pause()?,
                KeyCode::Esc => self.end_or_quit()?,
                KeyCode::Enter => self.mode = Mode::Prompting(self.open_prompt()),
                _ => {}
            },
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let clock = Local::now().format("%Y-%m-%d  %H:%M:%S").to_string();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(" hourus ").centered())
            .title_top(Line::from(format!(" {clock} ")).right_aligned())
            .border_style(Style::default().fg(Color::DarkGray));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Split into content area + one-line hint bar.
        let [content, hints] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

        match &self.mode {
            Mode::Normal => self.draw_normal(frame, content),
            Mode::Prompting(p) => self.draw_prompting(frame, content, p),
        }
        self.draw_hints(frame, hints);
    }

    fn draw_normal(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = vec![Line::from("")];

        match &self.session {
            SessionState::Active { desc, started_at } => {
                let elapsed = Local::now().naive_local() - *started_at;
                lines.push(Line::from(vec![
                    Span::styled("  ● ", Style::default().fg(Color::Green)),
                    Span::styled(desc.clone(), Style::default().add_modifier(Modifier::BOLD)),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("    started "),
                    Span::styled(
                        started_at.format("%H:%M:%S").to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw("  ·  running "),
                    Span::styled(format_duration(elapsed), Style::default().fg(Color::Green)),
                ]));
            }
            SessionState::Paused { desc } => {
                lines.push(Line::from(vec![
                    Span::styled("  ⏸ ", Style::default().fg(Color::Yellow)),
                    Span::styled(desc.clone(), Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled("  (paused)", Style::default().fg(Color::DarkGray)),
                ]));
                lines.push(Line::from(Span::styled(
                    "    press space to resume",
                    Style::default().fg(Color::DarkGray),
                )));
            }
            SessionState::Idle => {
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
                format_duration(self.duration_today()),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        frame.render_widget(Paragraph::new(lines), area);
    }

    fn draw_prompting(&self, frame: &mut Frame, area: Rect, p: &PromptState) {
        // 2 rows of top padding, then 3 rows for the textarea (border + 1 input line).
        let [_, ta_area, _] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .areas(area);

        // 2-column gutter on each side.
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
            Mode::Normal => match &self.session {
                SessionState::Active { .. } => Line::from(vec![
                    key("space"),
                    txt("pause"),
                    sep(),
                    key("esc"),
                    txt("end"),
                    sep(),
                    key("enter"),
                    txt("new task"),
                    sep(),
                    key("q"),
                    txt("quit"),
                ]),
                SessionState::Paused { .. } => Line::from(vec![
                    key("space"),
                    txt("resume"),
                    sep(),
                    key("esc"),
                    txt("discard"),
                    sep(),
                    key("enter"),
                    txt("new task"),
                    sep(),
                    key("q"),
                    txt("quit"),
                ]),
                SessionState::Idle => Line::from(vec![
                    key("enter"),
                    txt("start"),
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

pub fn run(storage: FileStorage, initial_desc: Option<String>) -> Result<()> {
    let mut app = App::new(storage)?;
    if let Some(desc) = initial_desc {
        app.start_session(&desc, Local::now().naive_local())?;
    }
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}
