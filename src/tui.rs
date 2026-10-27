use crate::session::{self, AgentSession, Status, fmt_age};
use crate::tmux;
use anyhow::Result;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Cell, Paragraph, Row, Table, TableState},
};
use std::time::{Duration, Instant};

pub struct Options {
    pub interval: Duration,
    pub preview: bool,
    pub stay: bool,
}

/// What to do once the terminal has been restored.
enum Exit {
    Quit,
    /// Not inside tmux: exec `tmux attach` on this target after cleanup.
    Attach {
        pane_id: String,
        session: String,
    },
}

struct App {
    agents: Vec<AgentSession>,
    state: TableState,
    preview: bool,
    error: Option<String>,
    flash: Option<(String, Instant)>,
    last_refresh: Instant,
}

impl App {
    fn selected(&self) -> Option<&AgentSession> {
        self.state.selected().and_then(|i| self.agents.get(i))
    }

    fn refresh(&mut self) {
        // Keep the cursor on the same pane even when sorting reshuffles rows.
        let keep = self.selected().map(|a| a.pane_id.clone());
        match session::scan() {
            Ok(a) => {
                self.agents = a;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        let idx = keep
            .and_then(|id| self.agents.iter().position(|a| a.pane_id == id))
            .or(if self.agents.is_empty() {
                None
            } else {
                Some(0)
            });
        let idx = idx.map(|i| i.min(self.agents.len().saturating_sub(1)));
        self.state
            .select(if self.agents.is_empty() { None } else { idx });
        self.last_refresh = Instant::now();
    }

    fn move_by(&mut self, delta: isize) {
        if self.agents.is_empty() {
            return;
        }
        let n = self.agents.len() as isize;
        let i = self.state.selected().unwrap_or(0) as isize;
        self.state
            .select(Some(((i + delta).rem_euclid(n)) as usize));
    }

    /// Jump to the next agent (after the cursor) that is waiting on you.
    fn next_needing_input(&mut self) {
        let n = self.agents.len();
        let start = self.state.selected().map_or(0, |i| i + 1);
        if let Some(i) = (0..n)
            .map(|k| (start + k) % n)
            .find(|&i| self.agents[i].status == Status::NeedsInput)
        {
            self.state.select(Some(i));
        } else {
            self.flash("nothing is waiting for input");
        }
    }

    fn flash(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), Instant::now()));
    }
}

pub fn run(opts: Options) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &opts);
    ratatui::restore();
    match result? {
        Exit::Quit => Ok(()),
        Exit::Attach { pane_id, session } => Err(tmux::exec_attach(&pane_id, &session)),
    }
}

fn event_loop(terminal: &mut DefaultTerminal, opts: &Options) -> Result<Exit> {
    let mut app = App {
        agents: vec![],
        state: TableState::default(),
        preview: opts.preview,
        error: None,
        flash: None,
        last_refresh: Instant::now(),
    };
    app.refresh();

    loop {
        terminal.draw(|f| draw(f, &mut app))?;

        let timeout = opts.interval.saturating_sub(app.last_refresh.elapsed());
        if event::poll(timeout)? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match (key.code, key.modifiers) {
                (KeyCode::Char('q') | KeyCode::Esc, _) => return Ok(Exit::Quit),
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(Exit::Quit),
                (KeyCode::Down | KeyCode::Char('j'), _) => app.move_by(1),
                (KeyCode::Up | KeyCode::Char('k'), _) => app.move_by(-1),
                (KeyCode::Home | KeyCode::Char('g'), _) => app.state.select_first(),
                (KeyCode::End | KeyCode::Char('G'), _) => {
                    if !app.agents.is_empty() {
                        app.state.select(Some(app.agents.len() - 1));
                    }
                }
                (KeyCode::Tab | KeyCode::Char('n'), _) => app.next_needing_input(),
                (KeyCode::Char('p'), _) => app.preview = !app.preview,
                (KeyCode::Char('r'), _) => {
                    app.refresh();
                    app.flash("refreshed");
                }
                (KeyCode::Char(c @ '1'..='9'), _) => {
                    let i = c as usize - '1' as usize;
                    if i < app.agents.len() {
                        app.state.select(Some(i));
                        if let Some(exit) = jump(&mut app, opts)? {
                            return Ok(exit);
                        }
                    }
                }
                (KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right, _) => {
                    if let Some(exit) = jump(&mut app, opts)? {
                        return Ok(exit);
                    }
                }
                _ => {}
            }
        } else {
            app.refresh();
        }
    }
}

/// Switch to the selected agent's pane. Returns `Some(exit)` when the TUI should close.
fn jump(app: &mut App, opts: &Options) -> Result<Option<Exit>> {
    let Some(a) = app.selected().cloned() else {
        return Ok(None);
    };
    if !tmux::inside_tmux() {
        return Ok(Some(Exit::Attach {
            pane_id: a.pane_id,
            session: a.session,
        }));
    }
    match tmux::switch_to(&a.pane_id) {
        Ok(()) if opts.stay => {
            app.flash(format!("switched to {}", a.target()));
            Ok(None)
        }
        Ok(()) => Ok(Some(Exit::Quit)),
        Err(e) => {
            app.flash(format!("switch failed: {e}"));
            Ok(None)
        }
    }
}

fn status_style(s: Status) -> Style {
    match s {
        Status::NeedsInput => Style::new().fg(Color::Black).bg(Color::Yellow).bold(),
        Status::Working => Style::new().fg(Color::Cyan).bold(),
        Status::Done => Style::new().fg(Color::Green).bold(),
        Status::Idle => Style::new().fg(Color::DarkGray),
    }
}

fn status_icon(s: Status) -> &'static str {
    match s {
        Status::NeedsInput => "●",
        Status::Working => "◐",
        Status::Done => "✓",
        Status::Idle => "○",
    }
}

fn draw(f: &mut Frame, app: &mut App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_header(f, header, app);

    if app.preview && app.selected().is_some() {
        let [table_area, preview_area] =
            Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(body);
        draw_table(f, table_area, app);
        draw_preview(f, preview_area, app);
    } else {
        draw_table(f, body, app);
    }

    draw_footer(f, footer, app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let count = |s| app.agents.iter().filter(|a| a.status == s).count();
    let mut spans = vec![
        Span::styled(
            " agentop ",
            Style::new().fg(Color::Black).bg(Color::Blue).bold(),
        ),
        Span::raw(format!("  {} agents   ", app.agents.len())),
    ];
    for s in [
        Status::NeedsInput,
        Status::Working,
        Status::Done,
        Status::Idle,
    ] {
        let n = count(s);
        if n > 0 {
            spans.push(Span::styled(
                format!("{} {} {}", status_icon(s), n, s.label().to_lowercase()),
                status_style(s),
            ));
            spans.push(Span::raw("   "));
        }
    }
    f.render_widget(Line::from(spans), area);
}

fn draw_table(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(" tmux agents ")
        .border_style(Style::new().fg(Color::DarkGray));

    if let Some(err) = &app.error {
        f.render_widget(
            Paragraph::new(format!("error: {err}")).red().block(block),
            area,
        );
        return;
    }
    if app.agents.is_empty() {
        let msg = "No Claude Code agents found in any tmux session.\n\nStart `claude` in a tmux pane and it will appear here.";
        f.render_widget(Paragraph::new(msg).dark_gray().block(block), area);
        return;
    }

    let header = Row::new([
        "#",
        "STATUS",
        "AGENT",
        "SESSION",
        "CONTEXT",
        "AGE",
        "DIRECTORY",
        "DETAIL",
    ])
    .style(Style::new().fg(Color::Gray).add_modifier(Modifier::BOLD))
    .bottom_margin(0);

    let rows = app.agents.iter().enumerate().map(|(i, a)| {
        let session = if a.session_attached {
            Line::from(vec![
                Span::raw(a.session.clone()),
                Span::styled(" *", Style::new().fg(Color::DarkGray)),
            ])
        } else {
            Line::from(a.session.clone())
        };
        Row::new(vec![
            Cell::from(if i < 9 {
                format!("{}", i + 1)
            } else {
                String::new()
            })
            .dark_gray(),
            Cell::from(format!("{} {}", status_icon(a.status), a.status.label()))
                .style(status_style(a.status)),
            Cell::from(a.kind).dark_gray(),
            Cell::from(session),
            Cell::from(context_text(a)),
            Cell::from(a.since_secs.map(fmt_age).unwrap_or_else(|| "-".into())).dark_gray(),
            Cell::from(a.cwd.clone()).blue(),
            Cell::from(a.detail.clone()),
        ])
    });

    let widths = [
        Constraint::Length(2),
        Constraint::Length(13),
        Constraint::Length(7),
        Constraint::Max(18),
        Constraint::Length(9),
        Constraint::Length(4),
        Constraint::Max(32),
        Constraint::Fill(1),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(Style::new().add_modifier(Modifier::BOLD))
        .highlight_symbol(Line::styled("▶ ", Style::new().fg(Color::Blue).bold()));

    f.render_stateful_widget(table, area, &mut app.state);
}

fn context_text(a: &AgentSession) -> String {
    session::context_text(a)
}

fn draw_preview(f: &mut Frame, area: Rect, app: &App) {
    let Some(a) = app.selected() else { return };
    let inner_h = area.height.saturating_sub(2) as usize;
    let start = a.screen.len().saturating_sub(inner_h);
    let text: Vec<Line> = a.screen[start..]
        .iter()
        .map(|l| Line::raw(l.clone()))
        .collect();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(format!(" {}  {} ", a.target(), a.cwd))
        .border_style(
            status_style(a.status)
                .remove_modifier(Modifier::BOLD)
                .bg(Color::Reset),
        );
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    if let Some((msg, at)) = &app.flash
        && at.elapsed() < Duration::from_secs(3)
    {
        f.render_widget(Line::from(format!(" {msg}")).yellow(), area);
        return;
    }
    let key = |k: &str| {
        Span::styled(
            format!(" {k} "),
            Style::new().fg(Color::Black).bg(Color::Gray),
        )
    };
    let label = |l: &str| Span::styled(format!(" {l}  "), Style::new().fg(Color::DarkGray));
    let enter_label = if tmux::inside_tmux() {
        "switch"
    } else {
        "attach"
    };
    f.render_widget(
        Line::from(vec![
            key("↑↓/jk"),
            label("move"),
            key("⏎"),
            label(enter_label),
            key("1-9"),
            label("jump"),
            key("tab"),
            label("next waiting"),
            key("p"),
            label("preview"),
            key("r"),
            label("refresh"),
            key("q"),
            label("quit"),
        ]),
        area,
    );
}
