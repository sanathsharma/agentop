mod agents;
mod args;
mod notify;
mod session;
mod state;
mod tmux;
mod tui;

use anyhow::Result;
use session::Status;
use std::time::Duration;

fn main() -> Result<()> {
    let matches = args::get_matches();

    match matches.subcommand() {
        None => {
            let interval = matches.get_one::<u64>("interval").copied().unwrap_or(1000);
            tui::run(tui::Options {
                interval: Duration::from_millis(interval.max(200)),
                preview: !matches.get_flag("no_preview"),
                stay: matches.get_flag("stay"),
            })
        }
        Some(("list", sub)) => list(sub.get_flag("json")),
        Some(("next", _)) => next(),
        Some(("hook", sub)) => {
            // Hooks must never break the agent: swallow errors.
            if let Ok(Some((pane, st))) = state::record_from_stdin() {
                let mode = sub
                    .get_one::<String>("notify")
                    .map_or(notify::Mode::Off, |m| notify::Mode::parse(m));
                if let Some(kind) = notify::classify(mode, &st.event)
                    && let Some(info) = notify::pane_info(&pane)
                    && !info.in_view
                {
                    notify::notify(
                        kind,
                        "claude",
                        &info.session,
                        st.message.as_deref().unwrap_or(""),
                        sub.get_flag("sound"),
                    );
                }
            }
            Ok(())
        }
        Some(("init", _)) => {
            print!("{}", init_text());
            Ok(())
        }
        Some(_) => Ok(()),
    }
}

fn list(json: bool) -> Result<()> {
    let sessions = session::scan()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!("no agents found");
        return Ok(());
    }
    println!(
        "{:<12} {:<8} {:<30} {:<5} {:<24} {:<13} {:<5} DETAIL",
        "STATUS", "AGENT", "TARGET", "PANE", "TITLE", "CONTEXT", "AGE"
    );
    for a in sessions {
        println!(
            "{:<12} {:<8} {:<30} {:<5} {:<24} {:<13} {:<5} {}",
            a.status.label(),
            a.kind,
            a.target(),
            a.pane_id,
            session::title_text(&a),
            session::context_text(&a),
            a.since_secs
                .map(session::fmt_age)
                .unwrap_or_else(|| "-".into()),
            a.detail
        );
    }
    Ok(())
}

fn next() -> Result<()> {
    let sessions = session::scan()?;
    let Some(a) = sessions.iter().find(|a| a.status == Status::NeedsInput) else {
        if tmux::inside_tmux() {
            let _ = tmux::tmux(&["display-message", "agentop: no agent is waiting for input"]);
        } else {
            eprintln!("no agent is waiting for input");
        }
        return Ok(());
    };
    if tmux::inside_tmux() {
        tmux::switch_to(&a.pane_id)
    } else {
        Err(tmux::exec_attach(&a.pane_id, &a.session))
    }
}

fn init_text() -> String {
    let mut out = String::from(TMUX_TEXT);
    for agent in agents::AGENTS {
        if let Some(text) = agent.init_text() {
            out.push('\n');
            out.push_str(text);
        }
    }
    out
}

const TMUX_TEXT: &str = r#"# tmux: add to ~/.tmux.conf, then `tmux source ~/.tmux.conf`
#    prefix + a : open the dashboard in a popup (Enter switches, popup closes)
#    prefix + A : jump straight to the next agent waiting for input
bind "a" display-popup -E -w 90% -h 90% "agentop"
bind "A" run-shell "agentop next"
# Needed for notifications: lets tmux know when the terminal window has focus,
# so agentop only skips a notification while you are actually looking at it.
set -g focus-events on
"#;
