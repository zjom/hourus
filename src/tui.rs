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
use ratatui_textarea::TextArea;
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

enum Mode {
    Normal,
    Prompting(TextArea<'static>),
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct App {
    session: SessionState,
    mode: Mode,
    storage: FileStorage,
    /// Completed-session time in the past 24 h, updated whenever a session ends/pauses.
    base_duration_today: TimeDelta,
    exit: bool,
}

impl App {
    fn new(storage: FileStorage) -> Result<Self> {
        let lines = storage.load()?;

        // Determine current session state from the last entry line.
        let state_report = Report::new().with_lines(lines.clone()).build()?;
        let session = match state_report.entry_lines.last() {
            Some(last) if last.kind == EntryKind::Start => SessionState::Active {
                desc: last.desc.clone(),
                started_at: last.dt,
            },
            _ => SessionState::Idle,
        };

        // Pre-compute completed session time for the past 24 h.
        let since_24h = Local::now().naive_local() - TimeDelta::hours(24);
        let base_duration_today = Report::new()
            .with_lines(lines)
            .from(since_24h)
            .build()?
            .total_duration();

        Ok(App {
            session,
            mode: Mode::Normal,
            storage,
            base_duration_today,
            exit: false,
        })
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

    fn new_textarea() -> TextArea<'static> {
        let mut ta = TextArea::default();
        ta.set_placeholder_text("task description...");
        ta.set_cursor_line_style(Style::default());
        ta.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" new session "),
        );
        ta
    }

    fn append_line(&mut self, kind: EntryKind, dt: NaiveDateTime, desc: &str) -> Result<()> {
        Ok(self.storage.append(&[EntryLine {
            kind,
            dt,
            desc: desc.to_owned(),
        }])?)
    }

    /// Account for completed session time, then update `base_duration_today`.
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
        // Move session out to avoid borrow-checker conflicts when calling &mut self methods.
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

    /// Esc: end the active session, discard a paused one, or quit if idle.
    fn end_or_quit(&mut self) -> Result<()> {
        let now = Local::now().naive_local();
        let prev = std::mem::replace(&mut self.session, SessionState::Idle);
        match prev {
            SessionState::Active { desc, started_at } => {
                self.append_line(EntryKind::End, now, &desc)?;
                self.accrue(started_at, now);
                // session stays Idle
            }
            SessionState::Paused { .. } => {
                // Discard the paused state; session stays Idle.
            }
            SessionState::Idle => {
                self.session = SessionState::Idle;
                self.exit = true;
            }
        }
        Ok(())
    }

    /// Start a new session with the given desc, ending the current one if necessary.
    fn start_session(&mut self, desc: &str, now: NaiveDateTime) -> Result<()> {
        let prev = std::mem::replace(&mut self.session, SessionState::Idle);
        // If something was running, end it first.
        let new_start = match prev {
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
        self.append_line(EntryKind::Start, new_start, desc)?;
        self.session = SessionState::Active {
            desc: desc.to_owned(),
            started_at: new_start,
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
            Mode::Prompting(mut ta) => {
                match key.code {
                    KeyCode::Esc => { /* mode stays Normal – cancels the prompt */ }
                    KeyCode::Enter => {
                        let desc = ta
                            .lines()
                            .first()
                            .map(|l| l.trim().to_lowercase())
                            .unwrap_or_default();
                        if !desc.is_empty() {
                            self.start_session(&desc, Local::now().naive_local())?;
                            // mode stays Normal
                        } else {
                            // Empty input: put prompting mode back unchanged.
                            self.mode = Mode::Prompting(ta);
                        }
                    }
                    _ => {
                        ta.input(key);
                        self.mode = Mode::Prompting(ta);
                    }
                }
            }
            Mode::Normal => match key.code {
                KeyCode::Char('q') => self.exit = true,
                KeyCode::Char(' ') => self.toggle_pause()?,
                KeyCode::Esc => self.end_or_quit()?,
                KeyCode::Enter => self.mode = Mode::Prompting(Self::new_textarea()),
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
            Mode::Prompting(ta) => self.draw_prompting(frame, content, ta),
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

    fn draw_prompting(&self, frame: &mut Frame, area: Rect, ta: &TextArea) {
        // Vertically: 2 rows of padding, then 3 rows for the textarea (border + 1 line).
        let [_, textarea_area, _] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .areas(area);

        // Horizontal: 2-column gutter on each side.
        let [_, ta_inner, _] = Layout::horizontal([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .areas(textarea_area);

        frame.render_widget(ta, ta_inner);
    }

    fn draw_hints(&self, frame: &mut Frame, area: Rect) {
        let style_key = Style::default().bg(Color::DarkGray).fg(Color::White);
        let style_sep = Style::default().fg(Color::DarkGray);

        let sep = || Span::styled("  ", style_sep);
        let key = |label: &'static str| Span::styled(format!(" {label} "), style_key);
        let text = |label: &'static str| Span::raw(format!(" {label}"));

        let hints: Line = match &self.mode {
            Mode::Prompting(_) => Line::from(vec![
                key("enter"),
                text("confirm"),
                sep(),
                key("esc"),
                text("cancel"),
            ]),
            Mode::Normal => match &self.session {
                SessionState::Active { .. } => Line::from(vec![
                    key("space"),
                    text("pause"),
                    sep(),
                    key("esc"),
                    text("end"),
                    sep(),
                    key("enter"),
                    text("new task"),
                    sep(),
                    key("q"),
                    text("quit"),
                ]),
                SessionState::Paused { .. } => Line::from(vec![
                    key("space"),
                    text("resume"),
                    sep(),
                    key("esc"),
                    text("discard"),
                    sep(),
                    key("enter"),
                    text("new task"),
                    sep(),
                    key("q"),
                    text("quit"),
                ]),
                SessionState::Idle => Line::from(vec![
                    key("enter"),
                    text("start"),
                    sep(),
                    key("esc"),
                    text("quit"),
                    sep(),
                    key("q"),
                    text("quit"),
                ]),
            },
        };

        frame.render_widget(Paragraph::new(hints), area);
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(storage: FileStorage) -> Result<()> {
    let mut app = App::new(storage)?;
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}
