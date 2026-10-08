//! gray-checkpoint — git stash checkpoints per session.
//!
//! Port of pi's `git-checkpoint` extension. pi snapped a `git stash create`
//! commit on every turn_start so /fork could roll code back; gray's wire has
//! no fork event, so this sidecar snapshots on `agent/before_start` and after
//! every `turn_end`, appending `{turn, ref}` lines to
//! `~/.gray/checkpoint/<session-id>.jsonl`. Restore is manual:
//! `/checkpoint list` shows them, `/checkpoint restore <n>` runs
//! `git stash apply <ref>` in the session's repo.
//!
//! Wire methods: plugin/manifest, agent/before_start, command/run,
//! plugin/shutdown; notification event/notify (turn_end).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

fn manifest() -> Value {
    json!({
        "name": "checkpoint",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": "2.0",
        "tools": [],
        "commands": ["/checkpoint"],
        "hooks": ["agent/before_start"],
    })
}

/// `~/.gray/checkpoint`, honoring `$GRAY_HOME` first.
fn state_dir() -> PathBuf {
    std::env::var_os("GRAY_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".gray")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("checkpoint")
}

/// Session ids are arbitrary strings; keep them filename-safe.
fn sanitize(sid: &str) -> String {
    sid.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' })
        .collect()
}

fn session_file(dir: &Path, sid: &str) -> PathBuf {
    dir.join(format!("{}.jsonl", sanitize(sid)))
}

fn git(cwd: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
}

fn in_repo(cwd: &str) -> bool {
    !cwd.is_empty()
        && git(cwd, &["rev-parse", "--git-dir"])
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Extract (session id, cwd) from a request's `params.session`.
fn session(params: &Value) -> (String, String) {
    let s = params.get("session").cloned().unwrap_or(Value::Null);
    (
        s.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
        s.get("cwd").and_then(Value::as_str).unwrap_or("").to_string(),
    )
}

fn read_entries(dir: &Path, sid: &str) -> Vec<(u64, String)> {
    std::fs::read_to_string(session_file(dir, sid))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            Some((
                v.get("turn").and_then(Value::as_u64)?,
                v.get("ref").and_then(Value::as_str)?.to_string(),
            ))
        })
        .collect()
}

/// `git stash create` in the session's repo; append `{turn, ref}` when it
/// yields a commit. Silently skips non-repos, missing ids, and clean trees
/// (stash create prints nothing when there's no diff).
fn record(dir: &Path, sid: &str, cwd: &str) {
    if sid.is_empty() || !in_repo(cwd) {
        return;
    }
    let Ok(out) = git(cwd, &["stash", "create"]) else { return };
    if !out.status.success() {
        return;
    }
    let r = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if r.is_empty() {
        return;
    }
    let turn = read_entries(dir, sid).len() as u64 + 1;
    let line = json!({ "turn": turn, "ref": r }).to_string();
    if std::fs::create_dir_all(dir).is_ok() {
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(session_file(dir, sid))
            .and_then(|mut f| writeln!(f, "{line}"));
    }
}

fn short(r: &str) -> &str {
    &r[..7.min(r.len())]
}

/// `/checkpoint …` — `argv` excludes the command name.
fn run_command(dir: &Path, argv: &[&str], sid: &str, cwd: &str) -> String {
    match argv.first().copied() {
        Some("restore") => {
            let Some(n) = argv.get(1).and_then(|s| s.parse::<u64>().ok()) else {
                return "usage: /checkpoint restore <n>".into();
            };
            let Some(r) = read_entries(dir, sid)
                .into_iter()
                .find(|(t, _)| *t == n)
                .map(|(_, r)| r)
            else {
                return format!("no checkpoint {n} — /checkpoint list to see them");
            };
            if !in_repo(cwd) {
                return "not in a git repo — cannot restore".into();
            }
            match git(cwd, &["stash", "apply", &r]) {
                Ok(o) if o.status.success() => {
                    let detail = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    format!(
                        "restored checkpoint {n} ({}){}",
                        short(&r),
                        if detail.is_empty() { String::new() } else { format!(" — {detail}") }
                    )
                }
                Ok(o) => format!(
                    "restore of {n} failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
                Err(e) => format!("restore of {n} failed: {e}"),
            }
        }
        Some("list") | None => {
            let entries = read_entries(dir, sid);
            if entries.is_empty() {
                return "no checkpoints for this session yet".into();
            }
            let mut out = String::from("checkpoints (turn → stash ref):\n");
            for (t, r) in entries {
                out.push_str(&format!("  {t}  {r}\n"));
            }
            out.push_str("/checkpoint restore <n> applies one with `git stash apply`");
            out
        }
        Some(other) => format!("unknown subcommand {other:?} — /checkpoint [list|restore <n>]"),
    }
}

/// One request → `Some(reply)`, or `None` for notifications. The bool asks
/// the loop to exit after writing the reply.
fn handle(req: &Value) -> (Option<Value>, bool) {
    let dir = state_dir();
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let Some(id) = id else {
        if method == "event/notify"
            && params.get("type").and_then(Value::as_str) == Some("turn_end")
        {
            let (sid, cwd) = session(&params);
            record(&dir, &sid, &cwd);
        }
        return (None, method == "plugin/shutdown");
    };
    let result = match method {
        "plugin/manifest" => manifest(),
        "agent/before_start" => {
            let (sid, cwd) = session(&params);
            record(&dir, &sid, &cwd);
            json!({})
        }
        "command/run" => {
            let argv: Vec<&str> = params
                .get("argv")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let (sid, cwd) = session(&params);
            json!({ "text": run_command(&dir, &argv, &sid, &cwd) })
        }
        "plugin/shutdown" => return (Some(json!({ "id": id, "result": {} })), true),
        _ => {
            let error = json!({ "code": -32601, "message": "method not found" });
            return (Some(json!({ "id": id, "error": error })), false);
        }
    };
    (Some(json!({ "id": id, "result": result })), false)
}

fn main() -> std::io::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("manifest") {
        println!("{}", manifest());
        return Ok(());
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let (reply, exit) = handle(&req);
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
        if exit {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(method: &str, params: Value) -> Value {
        handle(&json!({ "id": 1, "method": method, "params": params }))
            .0
            .unwrap()
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "gray-checkpoint-test-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// git init + one committed file; returns the repo path.
    fn git_repo(tag: &str) -> PathBuf {
        let d = tmpdir(tag);
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&d)
                .args(args)
                .output()
                .unwrap()
        };
        assert!(run(&["init", "-q"]).status.success());
        assert!(run(&["config", "user.email", "t@t"]).status.success());
        assert!(run(&["config", "user.name", "t"]).status.success());
        std::fs::write(d.join("a.txt"), "one\n").unwrap();
        assert!(run(&["add", "a.txt"]).status.success());
        assert!(run(&["commit", "-qm", "init"]).status.success());
        d
    }

    #[test]
    fn manifest_declares_hooks_and_command() {
        let m = call("plugin/manifest", Value::Null)["result"].clone();
        assert_eq!(m["name"], "checkpoint");
        assert_eq!(m["protocol"], "2.0");
        assert_eq!(m["hooks"], json!(["agent/before_start"]));
        assert_eq!(m["commands"], json!(["/checkpoint"]));
        assert_eq!(m["tools"], json!([]));
    }

    #[test]
    fn before_start_outside_a_repo_is_a_noop() {
        let dir = tmpdir("state-norepo");
        let cwd = tmpdir("not-a-repo");
        record(&dir, "s1", cwd.to_str().unwrap());
        assert!(read_entries(&dir, "s1").is_empty());
    }

    #[test]
    fn checkpoints_number_up_from_one() {
        let state = tmpdir("state-repo");
        let repo = git_repo("repo");
        let cwd = repo.to_str().unwrap();
        // Clean tree → stash create prints nothing → no entry.
        record(&state, "s1", cwd);
        assert!(read_entries(&state, "s1").is_empty());
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        record(&state, "s1", cwd);
        std::fs::write(repo.join("a.txt"), "three\n").unwrap();
        record(&state, "s1", cwd);
        let e = read_entries(&state, "s1");
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].0, 1);
        assert_eq!(e[1].0, 2);
        assert_ne!(e[0].1, e[1].1);
        let _ = std::fs::remove_dir_all(&state);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn session_ids_are_filename_safe() {
        assert_eq!(sanitize("a/b\\c:d e"), "a_b_c_d_e");
    }

    #[test]
    fn restore_missing_checkpoint_denies() {
        let dir = tmpdir("state-empty");
        let msg = run_command(&dir, &["restore", "9"], "nope", "/tmp");
        assert!(msg.contains("no checkpoint 9"), "got: {msg}");
    }

    #[test]
    fn list_empty_reports_none() {
        let dir = tmpdir("state-list");
        let msg = run_command(&dir, &[], "fresh", "/tmp");
        assert!(msg.contains("no checkpoints"), "got: {msg}");
    }

    #[test]
    fn turn_end_notification_records_and_stays_silent() {
        let (reply, exit) = handle(&json!({
            "method": "event/notify",
            "params": {"type": "turn_end", "session": {"id": "x", "cwd": "/nonexistent"}}
        }));
        assert!(reply.is_none() && !exit);
    }

    #[test]
    fn shutdown_replies_then_exits() {
        let (reply, exit) = handle(&json!({ "id": 2, "method": "plugin/shutdown" }));
        assert!(reply.is_some() && exit);
    }
}
