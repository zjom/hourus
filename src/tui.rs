use crate::entry::{EntryKind, EntryLine};
use crate::history::StringHistoryList;
use crate::output::format_duration;
use crate::report::Report;
use crate::storage::{FileStorage, Storage};
use anyhow::Result;
use chrono::{Local, Months, NaiveDate, NaiveDateTime, TimeDelta};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use ratatui_textarea::{CursorMove, TextArea};
use std::rc::Rc;
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

    /// Returns the `from` datetime for this span relative to `now`.
    fn from_dt(self, now: NaiveDateTime) -> NaiveDateTime {
        match self {
            Self::All => NaiveDateTime::MIN,
            Self::Day1 => now - TimeDelta::days(1),
            Self::Day3 => now - TimeDelta::days(3),
            Self::Day7 => now - TimeDelta::days(7),
            Self::Month1 => now
                .date()
                .checked_sub_months(Months::new(1))
                .unwrap_or(NaiveDate::MIN)
                .and_time(now.time()),
            Self::Month3 => now
                .date()
                .checked_sub_months(Months::new(3))
                .unwrap_or(NaiveDate::MIN)
                .and_time(now.time()),
            Self::Year1 => now
                .date()
                .checked_sub_months(Months::new(12))
                .unwrap_or(NaiveDate::MIN)
                .and_time(now.time()),
        }
    }
}

// ---------------------------------------------------------------------------
// State machine
//
// `desc` is `Rc<str>` rather than `String` so that the active/paused session
// shares the same heap allocation as the entry in `history`.  Moving between
// Active and Paused is then a reference-count bump, not a string copy.
// ---------------------------------------------------------------------------

enum SessionState {
    Idle,
    Active {
        desc: Rc<str>,
        started_at: NaiveDateTime,
    },
    Paused {
        desc: Rc<str>,
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
    history: StringHistoryList,
    /// Completed-session time in the past 24 h, updated whenever a session ends/pauses.
    base_duration_today: TimeDelta,
    /// All entry lines ever loaded or appended, used for summary computation.
    all_lines: Vec<EntryLine>,
    /// Timespan for the task summary panel.
    summary_span: Timespan,
    /// Cached summary of *completed* sessions for `summary_span`.
    ///
    /// This covers only finished intervals and is therefore stable between
    /// state transitions. `summary_for_display` merges it with the currently
    /// running session's live duration at render time, which is cheap: it
    /// clones a small Vec of unique task names rather than all of `all_lines`.
    ///
    /// Rebuilt eagerly on any transition that changes completed sessions
    /// (pause, end, start-over-existing) and when `summary_span` changes.
    /// Never rebuilt inside the 250 ms render loop.
    summary_cache: Vec<(String, TimeDelta)>,
    exit: bool,
}

impl App {
    fn new(storage: FileStorage) -> Result<Self> {
        let lines = storage.load()?;

        let state_report = Report::new().with_lines(lines.clone()).build()?;

        // Determine current session state from the last entry line.
        let session = match state_report.entry_lines.last() {
            Some(last) if last.kind == EntryKind::Start => SessionState::Active {
                desc: Rc::from(last.desc.as_str()),
                started_at: last.dt,
            },
            _ => SessionState::Idle,
        };

        // Build deduped history from all START lines, most-recent-first.
        let history = Self::build_history(&state_report.entry_lines);

        // Pre-compute completed session time for the past 24 h.
        let since_today = Local::now()
            .naive_local()
            .date()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let base_duration_today = Report::new()
            .with_lines(lines.clone())
            .from(since_today)
            .build()?
            .total_duration();

        // Prime the summary cache for the default span (All).
        let summary_cache = Self::build_summary_cache(&lines, Timespan::All);

        Ok(App {
            session,
            mode: Mode::Normal,
            storage,
            history,
            base_duration_today,
            all_lines: lines,
            summary_span: Timespan::All,
            summary_cache,
            exit: false,
        })
    }

    /// Build a deduplicated list of task descriptions (most-recent-first) from entry lines.
    fn build_history(entry_lines: &[EntryLine]) -> StringHistoryList {
        entry_lines
            .iter()
            .filter(|l| l.kind == EntryKind::Start)
            .map(|l| l.desc.as_str())
            .collect()
    }

    /// Compute the completed-sessions summary for `span` over `lines`.
    /// Called only on state transitions and span changes, never during rendering.
    fn build_summary_cache(lines: &[EntryLine], span: Timespan) -> Vec<(String, TimeDelta)> {
        let from = span.from_dt(Local::now().naive_local());
        Report::new()
            .with_lines(lines.to_vec())
            .from(from)
            .build()
            .map(|r| r.summarize())
            .unwrap_or_default()
    }

    /// Rebuild `summary_cache` in place.  Called after any transition that
    /// changes the set of completed sessions, and when `summary_span` changes.
    fn rebuild_summary_cache(&mut self) {
        self.summary_cache = Self::build_summary_cache(&self.all_lines, self.summary_span);
    }

    /// Summary ready for the display, merging the cache with the live running duration.
    ///
    /// Clones only `summary_cache` (a small Vec of unique task names), not `all_lines`.
    fn summary_for_display(&self) -> Vec<(String, TimeDelta)> {
        let SessionState::Active { desc, started_at } = &self.session else {
            // No running session — the cache is the complete answer.
            return self.summary_cache.clone();
        };

        let now = Local::now().naive_local();
        // Clamp start to the span boundary so we don't count time outside the window.
        let effective_start = (*started_at).max(self.summary_span.from_dt(now));
        let running = (now - effective_start).max(TimeDelta::zero());

        let mut result = self.summary_cache.clone();
        match result.iter_mut().find(|(d, _)| d.as_str() == desc.as_ref()) {
            Some(entry) => entry.1 = entry.1 + running,
            None => result.push((desc.to_string(), running)),
        }
        result
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
        let entry = EntryLine {
            kind,
            dt,
            desc: desc.to_owned(),
        };
        self.storage.append(&[entry.clone()])?;
        self.all_lines.push(entry);
        Ok(())
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
                // A completed interval was added — rebuild the cache.
                self.rebuild_summary_cache();
                self.session = SessionState::Paused { desc }; // Rc move, no alloc
            }
            SessionState::Paused { desc } => {
                self.append_line(EntryKind::Start, now, &desc)?;
                // Resuming doesn't change completed sessions — cache stays valid.
                self.session = SessionState::Active {
                    desc, // Rc move, no alloc
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
                // A completed interval was added — rebuild the cache.
                self.rebuild_summary_cache();
            }
            SessionState::Paused { .. } => { /* discard, session stays Idle */ }
            SessionState::Idle => self.exit = true,
        }
        Ok(())
    }

    /// Start a new session with `desc`, ending the current one first if necessary.
    fn start_session(&mut self, desc: &str, now: NaiveDateTime) -> Result<()> {
        // Push to history first, then borrow the interned Rc so that
        // SessionState::Active shares the same allocation — no second copy.
        self.history.push_str(desc);
        let desc_rc: Rc<str> = self.history.iter().next().unwrap().clone(); // refcount bump only

        let prev = std::mem::replace(&mut self.session, SessionState::Idle);
        let start_dt = match prev {
            SessionState::Active {
                desc: old_desc,
                started_at,
            } => {
                self.append_line(EntryKind::End, now, &old_desc)?;
                self.accrue(started_at, now);
                // A completed interval was added — rebuild the cache.
                self.rebuild_summary_cache();
                now + TimeDelta::seconds(1)
            }
            _ => now,
        };
        self.append_line(EntryKind::Start, start_dt, &desc_rc)?;
        self.session = SessionState::Active {
            desc: desc_rc, // shared Rc, no heap allocation
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
                KeyCode::Char('s') => {
                    self.summary_span = self.summary_span.next();
                    // Span changed — completed-session totals are now over a different
                    // window, so the cache is stale.
                    self.rebuild_summary_cache();
                }
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
        // Horizontal split: session status on the left, task summary on the right.
        let [status_area, summary_area] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(area);

        // --- Left: session status ---
        let mut lines: Vec<Line> = vec![Line::from("")];

        match &self.session {
            SessionState::Active { desc, started_at } => {
                let elapsed = Local::now().naive_local() - *started_at;
                lines.push(Line::from(vec![
                    Span::styled("  ● ", Style::default().fg(Color::Green)),
                    Span::styled(
                        desc.to_string(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
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
                    Span::styled(
                        desc.to_string(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
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

        frame.render_widget(Paragraph::new(lines), status_area);

        // Compute the display summary once and pass it down, rather than
        // having draw_summary recompute it independently.
        let summary = self.summary_for_display();
        self.draw_summary(frame, summary_area, &summary);
    }

    fn draw_summary(&self, frame: &mut Frame, area: Rect, summary: &[(String, TimeDelta)]) {
        let mut lines: Vec<Line> = vec![];

        // Header row with span label.
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
            // +2 gap between name column and duration column
            let col_width = max_name + 2;

            for (desc, dur) in summary {
                let pad = col_width - desc.len();
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::raw(desc.clone()),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(format_duration(*dur), Style::default().fg(Color::Cyan)),
                ]));
            }

            // Total row.
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
                    key("s"),
                    txt("span"),
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
                    key("s"),
                    txt("span"),
                    sep(),
                    key("q"),
                    txt("quit"),
                ]),
                SessionState::Idle => Line::from(vec![
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
