//! Claude Code's permission questions, answered on the island.
//!
//! Every Claude Code config on this PC runs `floaty-ask.exe` as its
//! `PermissionRequest` hook (src/bin/floaty-ask.rs). The hook connects here
//! (127.0.0.1, a port and a secret written to `claude-ask.txt` in Floaty's
//! data folder, which only this Windows user can read), sends the request and
//! waits. The island shows a card with Allow, Always (when Claude Code offers
//! a rule) and Deny; a click goes back to the hook as its decision.
//!
//! Claude Code shows its own question in the terminal at the same time and
//! takes whichever answer comes first. An answer typed there doesn't end the
//! hook, so each request's thread also watches the session's transcript: once
//! the tool has its result (or the model wrote its next message) the
//! question is over and the card goes. The hook ending (Claude Code stopped
//! it, Esc), a quiet card (55 minutes; 2 for an unattended `claude -p`) and
//! a question Floaty can't answer (AskUserQuestion, plan approval) leave the
//! answer to the terminal: the hook prints nothing. Nothing here ever
//! answers by itself.

use crate::events::{AppEvent, EventSink};
use crate::state::ClaudeAsk;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What was clicked on a card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Allow,
    /// Allow, and keep the rule Claude Code suggested ("don't ask again").
    Always,
    Deny,
}

/// How long a card waits: an attended session's terminal asks too, so
/// there's no hurry; an unattended `claude -p` would otherwise be denied at
/// once, so it waits only a little.
const WAIT_ATTENDED: Duration = Duration::from_secs(55 * 60);
const WAIT_UNATTENDED: Duration = Duration::from_secs(120);
/// The longest request taken (a hook's JSON is a few KB; a Write can carry a
/// whole file).
const MAX_BODY: usize = 8 << 20;

static NEXT: AtomicU64 = AtomicU64::new(1);

/// The requests waiting for a click: their thread's end of the answer.
fn waiting() -> &'static Mutex<HashMap<u64, Sender<Choice>>> {
    static W: OnceLock<Mutex<HashMap<u64, Sender<Choice>>>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A card's button: hands the choice to the hook waiting for it. False
/// when that question is already over (answered in the terminal).
pub fn answer(id: u64, choice: Choice) -> bool {
    let tx = waiting().lock().ok().and_then(|mut w| w.remove(&id));
    tx.is_some_and(|tx| tx.send(choice).is_ok())
}

pub struct ClaudeAskService;

impl ClaudeAskService {
    pub fn start(sink: EventSink) -> Result<Self, String> {
        let l = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        let port = l.local_addr().map_err(|e| e.to_string())?.port();
        let token = secret().ok_or("no random secret")?;
        write_ticket(&crate::paths::data_dir(), port, &token).map_err(|e| format!("claude-ask.txt: {e}"))?;
        crate::pinfo!("claude ask: listening on 127.0.0.1:{port}");
        std::thread::Builder::new()
            .name("claude-ask".into())
            .spawn(move || {
                for conn in l.incoming().flatten() {
                    let (sink, token) = (sink.clone(), token.clone());
                    let _ = std::thread::Builder::new().name("claude-ask-req".into()).spawn(move || serve(conn, &token, &sink));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self)
    }
}

fn secret() -> Option<String> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_ALG_HANDLE, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut b = [0u8; 24];
    unsafe { BCryptGenRandom(BCRYPT_ALG_HANDLE::default(), &mut b, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.ok().ok()?;
    Some(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// "<port> <secret>" for the hook, written whole (temp file, then rename).
fn write_ticket(dir: &Path, port: u16, token: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("claude-ask.tmp");
    std::fs::write(&tmp, format!("{port} {token}\n"))?;
    std::fs::rename(&tmp, dir.join("claude-ask.txt"))
}

/// One hook's request: "<secret> <claude pid> <bytes> [attended]\n" and
/// the hook's JSON. Answers with the decision's JSON, or nothing.
fn serve(mut s: TcpStream, token: &str, sink: &EventSink) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.set_nodelay(true);
    let (pid, attended, body) = match read_request(&mut s, token) {
        Ok(r) => r,
        Err(e) => {
            crate::pwarn!("claude ask: bad request ({e})");
            return;
        }
    };
    let req = match serde_json::from_slice::<Value>(&body) {
        Ok(v) => v,
        Err(e) => {
            crate::pwarn!("claude ask: not JSON ({e})");
            return;
        }
    };
    let Some(mut card) = card_of(&req, pid) else {
        // Not something a click can answer: the terminal asks.
        let _ = s.shutdown(Shutdown::Both);
        return;
    };
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    card.id = id;
    let (tx, rx) = channel();
    if let Ok(mut w) = waiting().lock() {
        w.insert(id, tx);
    }
    crate::pinfo!("claude ask {id}: {} in {} ({})", card.tool, card.project, card.config);
    sink.send(AppEvent::ClaudeAsk(card));

    let mut watch = TxWatch::new(&req);
    let cap = if attended { WAIT_ATTENDED } else { WAIT_UNATTENDED };
    let start = Instant::now();
    let _ = s.set_nonblocking(true);
    let mut ticks = 0u32;
    let (choice, why) = loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(c) => break (Some(c), "clicked"),
            Err(RecvTimeoutError::Disconnected) => break (None, "dropped"),
            Err(RecvTimeoutError::Timeout) => {}
        }
        if hook_gone(&s) {
            break (None, "hook ended");
        }
        ticks += 1;
        if ticks % 3 == 0 && watch.answered() {
            break (None, "answered in the terminal");
        }
        if start.elapsed() > cap {
            break (None, "timed out");
        }
    };
    if let Ok(mut w) = waiting().lock() {
        w.remove(&id);
    }
    crate::pinfo!("claude ask {id}: {} ({why}, {:.1} s)", choice.map_or("no answer", |c| match c {
        Choice::Allow => "allow",
        Choice::Always => "always",
        Choice::Deny => "deny",
    }), start.elapsed().as_secs_f32());
    if choice.is_none() {
        sink.send(AppEvent::ClaudeAskGone(id));
    }
    let _ = s.set_nonblocking(false);
    if let Some(c) = choice {
        let _ = s.write_all(decision(c, &req).to_string().as_bytes());
        let _ = s.flush();
    }
    let _ = s.shutdown(Shutdown::Both);
}

fn read_request(s: &mut TcpStream, token: &str) -> Result<(u32, bool, Vec<u8>), String> {
    let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
    let mut head = String::new();
    r.by_ref().take(256).read_line(&mut head).map_err(|e| format!("head: {e}"))?;
    let mut it = head.split_whitespace();
    let tok = it.next().ok_or("no secret")?;
    let pid = it.next().and_then(|p| p.parse::<u32>().ok()).ok_or("no pid")?;
    let len = it.next().and_then(|p| p.parse::<usize>().ok()).ok_or("no length")?;
    let attended = it.next() != Some("0");
    if !poni_core::phonedrop::token::same(tok, token) {
        return Err("without the secret".into());
    }
    if len > MAX_BODY {
        return Err(format!("{len} bytes"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(|e| format!("body: {e}"))?;
    Ok((pid, attended, body))
}

/// The hook's end closed: Claude Code ended it (or it crashed).
fn hook_gone(s: &TcpStream) -> bool {
    let mut b = [0u8; 1];
    match s.peek(&mut b) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
    }
}

/// The hook's answer for Claude Code.
pub fn decision(c: Choice, req: &Value) -> Value {
    let d = match c {
        Choice::Allow => json!({ "behavior": "allow" }),
        Choice::Always => match req.get("permission_suggestions").filter(|v| v.as_array().is_some_and(|a| !a.is_empty())) {
            Some(rules) => json!({ "behavior": "allow", "updatedPermissions": rules }),
            None => json!({ "behavior": "allow" }),
        },
        Choice::Deny => json!({ "behavior": "deny", "message": "The user denied this from Floaty (the island)." }),
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": d } })
}

/// Tools whose question a plain Allow doesn't answer (they need the
/// person's own input in the terminal).
const TERMINAL_ONLY: [&str; 2] = ["AskUserQuestion", "ExitPlanMode"];

/// The card for a request: where, which tool, and what it would do.
pub fn card_of(req: &Value, pid: u32) -> Option<ClaudeAsk> {
    let tool = req.get("tool_name")?.as_str()?.to_string();
    if TERMINAL_ONLY.contains(&tool.as_str()) {
        return None;
    }
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("");
    let cwd = s("cwd");
    let project = Path::new(cwd.trim_end_matches(['\\', '/'])).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tx = s("transcript_path");
    let session = crate::services::claude::transcript_title_of(tx).unwrap_or_default();
    let always = req.get("permission_suggestions").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    Some(ClaudeAsk {
        id: 0,
        pid,
        session,
        project,
        config: crate::services::claude::config_name(tx),
        tool: tool_label(&tool),
        summary: summary(&tool, req.get("tool_input").unwrap_or(&Value::Null)),
        always,
    })
}

/// "Bash", "Edit", or "github · create_issue" for an MCP tool.
fn tool_label(tool: &str) -> String {
    match tool.strip_prefix("mcp__") {
        Some(rest) => {
            let mut p = rest.splitn(2, "__");
            let server = p.next().unwrap_or("");
            let name = p.next().unwrap_or("");
            let server = server.strip_prefix("claude_ai_").unwrap_or(server).replace('_', " ");
            if name.is_empty() {
                server
            } else {
                format!("{server} \u{b7} {name}")
            }
        }
        None => tool.to_string(),
    }
}

/// One line of what the tool would do: the command, the file, the address.
fn summary(tool: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    let line = match tool {
        "Bash" | "PowerShell" => s("command").map(first_line).or_else(|| s("description").map(str::to_string)),
        "Write" | "Edit" | "MultiEdit" | "Read" | "NotebookEdit" => s("file_path").or_else(|| s("notebook_path")).map(short_path),
        "WebFetch" => s("url").map(|u| u.trim_start_matches("https://").trim_start_matches("http://").to_string()),
        "WebSearch" => s("query").map(|q| format!("\u{201c}{q}\u{201d}")),
        "Glob" | "Grep" => s("pattern").map(str::to_string),
        "Agent" | "Task" => s("description").map(str::to_string),
        _ => None,
    };
    let line = line.or_else(|| {
        // Anything else: its first short text argument.
        input.as_object()?.values().filter_map(Value::as_str).map(first_line).find(|v| !v.is_empty())
    });
    clip(&line.unwrap_or_default(), 140)
}

fn first_line(s: &str) -> String {
    let mut lines = s.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or("").to_string();
    if lines.next().is_some() {
        format!("{first} \u{2026}")
    } else {
        first
    }
}

/// The last two parts of a path: "src\main.rs".
fn short_path(p: &str) -> String {
    let parts: Vec<&str> = p.split(['\\', '/']).filter(|x| !x.is_empty()).collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].to_string(),
        n => format!("{}\\{}", parts[n - 2], parts[n - 1]),
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('\u{2026}');
        t
    }
}

/// Watches the session's transcript for the end of this question: the
/// tool_use the request is about gets its tool_result (allowed and run, or
/// refused), or the model writes its next message.
struct TxWatch {
    path: PathBuf,
    tool: String,
    input: Value,
    /// The tool_use this question is about, once found, and the message
    /// it is in.
    found: Option<(String, String)>,
    len: u64,
}

impl TxWatch {
    fn new(req: &Value) -> Self {
        let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Self { path: PathBuf::from(s("transcript_path")), tool: s("tool_name"), input: req.get("tool_input").cloned().unwrap_or(Value::Null), found: None, len: 0 }
    }

    fn answered(&mut self) -> bool {
        let Ok(len) = std::fs::metadata(&self.path).map(|m| m.len()) else { return false };
        if len == self.len {
            return false;
        }
        self.len = len;
        let Some(tail) = read_tail(&self.path, 512 * 1024) else { return false };
        question_over(&tail, &self.tool, &self.input, &mut self.found)
    }
}

fn read_tail(path: &Path, max: u64) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let take = len.min(max);
    f.seek(SeekFrom::Start(len - take)).ok()?;
    let mut buf = Vec::with_capacity(take as usize);
    f.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Whether the transcript `tail` shows the question about `tool` (with
/// `input`) is over. `found` keeps the tool_use's id and its message's id
/// between calls: the newest tool_use of that tool with no result yet (the
/// one whose input matches when there are several).
fn question_over(tail: &str, tool: &str, input: &Value, found: &mut Option<(String, String)>) -> bool {
    let lines: Vec<Value> = tail.lines().filter(|l| l.contains("\"type\":\"assistant\"") || l.contains("tool_result")).filter_map(|l| serde_json::from_str(l).ok()).collect();
    let blocks = |v: &Value| v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut results = std::collections::HashSet::new();
    for v in &lines {
        for b in blocks(v) {
            if b.get("type").and_then(Value::as_str) == Some("tool_result") {
                if let Some(id) = b.get("tool_use_id").and_then(Value::as_str) {
                    results.insert(id.to_string());
                }
            }
        }
    }
    if found.is_none() {
        let mut open: Vec<(String, String, bool)> = Vec::new();
        for v in &lines {
            if v.get("type").and_then(Value::as_str) != Some("assistant") {
                continue;
            }
            let msg = v.pointer("/message/id").and_then(Value::as_str).unwrap_or("").to_string();
            for b in blocks(v) {
                if b.get("type").and_then(Value::as_str) != Some("tool_use") || b.get("name").and_then(Value::as_str) != Some(tool) {
                    continue;
                }
                let Some(id) = b.get("id").and_then(Value::as_str) else { continue };
                if !results.contains(id) {
                    let same = b.get("input").is_some_and(|i| same_input(i, input));
                    open.push((id.to_string(), msg.clone(), same));
                }
            }
        }
        *found = open.iter().rev().find(|o| o.2).or(open.last()).map(|(id, msg, _)| (id.clone(), msg.clone()));
    }
    let Some((id, msg)) = found.as_ref() else { return false };
    if results.contains(id) {
        return true;
    }
    // The model went on: a later assistant message than the tool_use's.
    let mut seen = false;
    for v in &lines {
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let m = v.pointer("/message/id").and_then(Value::as_str).unwrap_or("");
        if m == msg {
            seen = true;
        } else if seen && !m.is_empty() {
            return true;
        }
    }
    false
}

/// The hook's tool_input against the transcript's (Claude Code turns a
/// path's slashes into backslashes for the hook).
fn same_input(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x == y || x.replace('/', "\\") == y.replace('/', "\\"),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same_input(v, w))),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same_input(v, w)),
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(tool: &str, input: Value, suggestions: Value) -> Value {
        json!({
            "session_id": "s", "cwd": "C:\\Users\\k\\Videos\\Tools\\oyenza-studio",
            "transcript_path": "C:\\Users\\k\\.claude-loka\\projects\\p\\nope.jsonl",
            "hook_event_name": "PermissionRequest", "tool_name": tool, "tool_input": input,
            "permission_suggestions": suggestions
        })
    }

    #[test]
    fn a_card_says_where_and_what() {
        let r = req("Bash", json!({"command": "git push origin main\ngit status", "description": "Push"}), json!([{"type": "addRules"}]));
        let c = card_of(&r, 42).unwrap();
        assert_eq!((c.project.as_str(), c.config.as_str(), c.tool.as_str(), c.summary.as_str(), c.always, c.pid), ("oyenza-studio", "Loka", "Bash", "git push origin main \u{2026}", true, 42));
        let w = card_of(&req("Write", json!({"file_path": "C:\\a\\b\\src\\main.rs", "content": "x"}), json!([])), 0).unwrap();
        assert_eq!((w.summary.as_str(), w.always), ("src\\main.rs", false));
        let m = card_of(&req("mcp__claude_ai_Gmail__send_email", json!({"to": "a@b.c"}), json!([])), 0).unwrap();
        assert_eq!((m.tool.as_str(), m.summary.as_str()), ("Gmail \u{b7} send_email", "a@b.c"));
        assert!(card_of(&req("AskUserQuestion", json!({}), json!([])), 0).is_none());
    }

    #[test]
    fn decisions_are_claude_codes_shape() {
        let sugg = json!([{"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "git push:*"}], "behavior": "allow", "destination": "localSettings"}]);
        let r = req("Bash", json!({"command": "git push"}), sugg.clone());
        assert_eq!(decision(Choice::Allow, &r), json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {"behavior": "allow"}}}));
        assert_eq!(decision(Choice::Always, &r)["hookSpecificOutput"]["decision"]["updatedPermissions"], sugg);
        let d = decision(Choice::Deny, &r);
        assert_eq!(d["hookSpecificOutput"]["decision"]["behavior"], "deny");
        // Always without a suggestion is a plain allow.
        assert_eq!(decision(Choice::Always, &req("Bash", json!({}), json!([])))["hookSpecificOutput"]["decision"], json!({"behavior": "allow"}));
    }

    #[test]
    fn the_transcript_tells_when_the_question_is_over() {
        let input = json!({"file_path": "C:\\t\\hello.txt", "content": "hi"});
        let use_line = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"toolu_1","name":"Write","input":{"file_path":"C:/t/hello.txt","content":"hi"}}]}}"#;
        let mut found = None;
        assert!(!question_over(use_line, "Write", &input, &mut found));
        assert_eq!(found.as_ref().map(|f| f.0.as_str()), Some("toolu_1"));
        // A parallel Read's result doesn't end it.
        let other = format!("{use_line}\n{}", r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_9","content":"x"}]}}"#);
        assert!(!question_over(&other, "Write", &input, &mut found));
        // Its own result does.
        let done = format!("{use_line}\n{}", r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"denied"}]}}"#);
        assert!(question_over(&done, "Write", &input, &mut found));
        // So does the model's next message.
        let mut f2 = None;
        let next = format!("{use_line}\n{}", r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"ok"}]}}"#);
        assert!(question_over(&next, "Write", &input, &mut f2));
        // Its own message streamed in pieces is not a next one.
        let mut f3 = None;
        let same = format!("{use_line}\n{}", r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"more"}]}}"#);
        assert!(!question_over(&same, "Write", &input, &mut f3));
    }

    #[test]
    fn the_ticket_is_port_and_secret() {
        let dir = std::env::temp_dir().join(format!("poni-ask-{}", std::process::id()));
        write_ticket(&dir, 51234, "abc").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("claude-ask.txt")).unwrap(), "51234 abc\n");
        let _ = std::fs::remove_dir_all(dir);
    }
}
