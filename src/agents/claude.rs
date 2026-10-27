//! Claude Code.
//!
//! Status comes from the per-pid session file (`status` is `busy`, `waiting`
//! or `idle`). When that is missing or unrecognized, the pane title decides:
//! Claude Code prefixes it with a spinner glyph while working and `✳ ` when
//! idle. The screen contents are never used. Anything else is `Unknown`.
//!
//! Context usage comes from the session transcript. Claude Code records the
//! running session at `~/.claude/sessions/<pid>.json` (session id and cwd),
//! and the transcript at `~/.claude/projects/<cwd with non-alphanumerics as
//! '-'>/<session id>.jsonl`. The last assistant message there carries a
//! `usage` block. The window size is not recorded, so only used tokens are shown.

use super::{Agent, ContextUsage, ProcessInfo, SessionTitle, Status};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Transcript lines can be large (file reads), so look at the last megabyte.
const TAIL_BYTES: u64 = 1 << 20;

pub struct Claude;

impl Agent for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn matches(&self, args: &str) -> bool {
        is_claude(args)
    }

    fn classify(&self, proc: &ProcessInfo, title: &str) -> (Status, String) {
        let session = std::fs::read_to_string(session_path(proc.pid)).ok();
        let session = session.as_deref().and_then(parse_session_file);
        if let Some(s) = session
            && let Some(hit) = status_from_session(&s)
        {
            return hit;
        }
        (status_from_title(title), String::new())
    }

    fn session_title(&self, proc: &ProcessInfo, title: &str) -> Option<SessionTitle> {
        let file = std::fs::read_to_string(session_path(proc.pid)).ok();
        let file = file.as_deref().and_then(parse_session_file);
        let ai = config_dir()
            .and_then(|c| locate_transcript(&c, proc))
            .and_then(|t| read_tail(&t))
            .and_then(|tail| last_ai_title(&tail));
        title_from(file.as_ref(), ai, title)
    }

    fn context_usage(&self, proc: &ProcessInfo) -> Option<ContextUsage> {
        let transcript = locate_transcript(&config_dir()?, proc)?;
        let sample = last_usage(&read_tail(&transcript)?)?;
        Some(ContextUsage { used: sample.used })
    }

    fn init_text(&self) -> Option<&'static str> {
        Some(INIT_TEXT)
    }
}

fn is_claude(args: &str) -> bool {
    let mut it = args.split_whitespace();
    let Some(argv0) = it.next() else { return false };
    let base = argv0.rsplit('/').next().unwrap_or(argv0);
    if base == "claude" {
        return true;
    }
    // npm install runs as `node /…/@anthropic-ai/claude-code/cli.js`
    matches!(base, "node" | "bun" | "deno")
        && it.any(|a| a.contains("claude-code") || a.ends_with("/claude"))
}

fn session_path(pid: u32) -> PathBuf {
    config_dir()
        .unwrap_or_default()
        .join("sessions")
        .join(format!("{pid}.json"))
}

fn status_from_session(s: &SessionFile) -> Option<(Status, String)> {
    match s.status.as_deref()? {
        "busy" => Some((Status::Working, String::new())),
        "waiting" => Some((
            Status::NeedsInput,
            s.waiting_for.clone().unwrap_or_default(),
        )),
        "idle" => Some((Status::Idle, String::new())),
        _ => None,
    }
}

/// Spinner glyphs (braille, or half circles from 2.1.228) mean working and
/// `✳` means idle.
fn status_from_title(title: &str) -> Status {
    match title_parts(title) {
        Some(('\u{2800}'..='\u{28FF}' | '\u{25D0}'..='\u{25D3}', _)) => Status::Working,
        Some(('\u{2733}', _)) => Status::Idle,
        _ => Status::Unknown,
    }
}

/// Splits a pane title of the form `<glyph> <text>`.
fn title_parts(title: &str) -> Option<(char, &str)> {
    let mut chars = title.chars();
    let glyph = chars.next()?;
    (chars.next()? == ' ').then_some((glyph, chars.as_str()))
}

/// A name the user chose with `/rename` wins. Next comes the title Claude
/// generated from the conversation, then the generated name from the session
/// file, then the pane title. Only the last two are placeholders.
fn title_from(file: Option<&SessionFile>, ai: Option<String>, title: &str) -> Option<SessionTitle> {
    let file_name = file
        .and_then(|f| Some((f.name.as_deref().filter(|n| !n.is_empty())?, f)))
        .map(|(n, f)| (n.to_string(), f.name_source.as_deref() == Some("derived")));
    if let Some((text, false)) = file_name {
        return Some(SessionTitle {
            text,
            placeholder: false,
        });
    }
    let placeholder = |text: String| SessionTitle {
        text,
        placeholder: true,
    };
    if let Some(text) = ai {
        return Some(SessionTitle {
            text,
            placeholder: false,
        });
    }
    if let Some((text, _)) = file_name {
        return Some(placeholder(text));
    }
    let (_, text) = title_parts(title)?;
    (!text.is_empty()).then(|| placeholder(text.to_string()))
}

/// The newest `ai-title` record in the transcript tail.
fn last_ai_title(tail: &str) -> Option<String> {
    tail.lines()
        .rev()
        .filter(|l| l.contains("\"ai-title\""))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["type"] == "ai-title")
        .and_then(|v| v["aiTitle"].as_str().map(String::from))
        .filter(|t| !t.is_empty())
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude")))
}

/// Claude Code names a project directory after the cwd, replacing every
/// character that is not an ASCII letter or digit with `-`.
fn project_dir_name(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

struct SessionFile {
    session_id: String,
    cwd: String,
    status: Option<String>,
    waiting_for: Option<String>,
    name: Option<String>,
    name_source: Option<String>,
}

fn parse_session_file(json: &str) -> Option<SessionFile> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(SessionFile {
        session_id: v["sessionId"].as_str()?.to_string(),
        cwd: v["cwd"].as_str()?.to_string(),
        status: v["status"].as_str().map(String::from),
        waiting_for: v["waitingFor"].as_str().map(String::from),
        name: v["name"].as_str().map(String::from),
        name_source: v["nameSource"].as_str().map(String::from),
    })
}

fn newest_transcript(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// Find the transcript for the claude process. The per-pid session file is
/// exact. Without it, the newest transcript for the pane's cwd is a guess.
fn locate_transcript(config: &Path, proc: &ProcessInfo) -> Option<PathBuf> {
    let projects = config.join("projects");
    if let Ok(json) =
        std::fs::read_to_string(config.join("sessions").join(format!("{}.json", proc.pid)))
        && let Some(s) = parse_session_file(&json)
    {
        let path = projects
            .join(project_dir_name(&s.cwd))
            .join(format!("{}.jsonl", s.session_id));
        if path.exists() {
            return Some(path);
        }
    }
    newest_transcript(&projects.join(project_dir_name(&proc.cwd)))
}

fn read_tail(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if start > 0 {
        // The first line is probably cut in half.
        return text.split_once('\n').map(|(_, rest)| rest.to_string());
    }
    Some(text)
}

struct UsageSample {
    used: u64,
}

/// Tokens in the context after the most recent main-thread assistant message:
/// fresh input plus cache creation plus cache reads.
fn last_usage(transcript: &str) -> Option<UsageSample> {
    for line in transcript.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v["isSidechain"].as_bool().unwrap_or(false) {
            continue;
        }
        let msg = &v["message"];
        if msg["role"].as_str() != Some("assistant") || msg["model"].as_str() == Some("<synthetic>")
        {
            continue;
        }
        let usage = &msg["usage"];
        let used: u64 = [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ]
        .iter()
        .filter_map(|k| usage[*k].as_u64())
        .sum();
        if used > 0 {
            return Some(UsageSample { used });
        }
    }
    None
}

const INIT_TEXT: &str = r#"# Claude Code (optional, recommended): merge into ~/.claude/settings.json
#    Lets agentop show DONE vs IDLE, the age of the last event, and catch
#    permission requests even when the prompt is scrolled off screen.
#    Optional: add `--notify` (or `--notify=input`) and `--sound` to the
#    Notification and Stop commands for desktop notifications and sounds.
{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "agentop hook" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "agentop hook --notify --sound" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "agentop hook --notify --sound" }] }],
    "SessionEnd":       [{ "hooks": [{ "type": "command", "command": "agentop hook" }] }]
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_claude_processes() {
        assert!(is_claude("claude"));
        assert!(is_claude("/usr/local/bin/claude --resume"));
        assert!(is_claude(
            "node /usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"
        ));
        assert!(!is_claude("vim claude.md"));
        assert!(!is_claude("node server.js"));
        assert!(!is_claude("agentop"));
    }

    #[test]
    fn encodes_project_dir() {
        assert_eq!(
            project_dir_name("/home/u/code/dotfiles.git/dotfiles--main"),
            "-home-u-code-dotfiles-git-dotfiles--main"
        );
    }

    #[test]
    fn parses_session_file() {
        let s = parse_session_file(
            r#"{"pid":1,"sessionId":"abc-123","cwd":"/home/u/x","status":"idle"}"#,
        )
        .unwrap();
        assert_eq!(s.session_id, "abc-123");
        assert_eq!(s.cwd, "/home/u/x");
        assert!(parse_session_file("{}").is_none());
    }

    fn session(status: &str) -> SessionFile {
        parse_session_file(&format!(
            r#"{{"sessionId":"a","cwd":"/x","status":"{status}","waitingFor":"input needed"}}"#
        ))
        .unwrap()
    }

    #[test]
    fn maps_session_status() {
        assert_eq!(
            status_from_session(&session("busy")).unwrap().0,
            Status::Working
        );
        let (st, d) = status_from_session(&session("waiting")).unwrap();
        assert_eq!(st, Status::NeedsInput);
        assert_eq!(d, "input needed");
        assert_eq!(
            status_from_session(&session("idle")).unwrap().0,
            Status::Idle
        );
        assert!(status_from_session(&session("other")).is_none());
    }

    #[test]
    fn maps_title() {
        assert_eq!(status_from_title("⠋ Fix bug"), Status::Working);
        assert_eq!(status_from_title("◐ Fix bug"), Status::Working);
        assert_eq!(status_from_title("✳ Fix bug"), Status::Idle);
        assert_eq!(status_from_title("~/code/x"), Status::Unknown);
        assert_eq!(status_from_title(""), Status::Unknown);
    }

    #[test]
    fn picks_session_title() {
        let file = |src: &str| {
            parse_session_file(&format!(
                r#"{{"sessionId":"a","cwd":"/x","name":"n","nameSource":"{src}"}}"#
            ))
            .unwrap()
        };
        let ai = || Some("Fix bug".to_string());

        let t = title_from(Some(&file("user")), ai(), "✳ Other").unwrap();
        assert_eq!((t.text.as_str(), t.placeholder), ("n", false));

        let t = title_from(Some(&file("derived")), ai(), "").unwrap();
        assert_eq!((t.text.as_str(), t.placeholder), ("Fix bug", false));

        let t = title_from(Some(&file("derived")), None, "✳ Other").unwrap();
        assert_eq!(t.text, "n");

        let t = title_from(None, None, "✳ Other").unwrap();
        assert_eq!(t.text, "Other");
        assert!(title_from(None, None, "~/code/x").is_none());
    }

    #[test]
    fn finds_last_ai_title() {
        let t = concat!(
            r#"{"type":"ai-title","aiTitle":"Old"}"#,
            "\n",
            r#"{"type":"user","message":{"content":"ai-title"}}"#,
            "\n",
            r#"{"type":"ai-title","aiTitle":"New"}"#,
            "\n",
        );
        assert_eq!(last_ai_title(t).as_deref(), Some("New"));
        assert!(last_ai_title("").is_none());
    }

    #[test]
    fn sums_last_main_thread_usage() {
        let t = concat!(
            r#"{"message":{"role":"assistant","model":"claude-sonnet-5-5","usage":{"input_tokens":1,"cache_creation_input_tokens":10,"cache_read_input_tokens":100}}}"#,
            "\n",
            r#"{"message":{"role":"assistant","model":"claude-sonnet-5-5","usage":{"input_tokens":2,"cache_creation_input_tokens":612,"cache_read_input_tokens":50790,"output_tokens":386}}}"#,
            "\n",
            r#"{"isSidechain":true,"message":{"role":"assistant","model":"claude-haiku-5-5","usage":{"input_tokens":999999}}}"#,
            "\n",
            r#"{"message":{"role":"assistant","model":"<synthetic>","usage":{"input_tokens":0}}}"#,
            "\n",
            r#"{"message":{"role":"user","content":"hi"}}"#,
            "\n",
            "not json at all\n",
        );
        let s = last_usage(t).unwrap();
        assert_eq!(s.used, 51_404);
        assert!(last_usage("").is_none());
    }
}
