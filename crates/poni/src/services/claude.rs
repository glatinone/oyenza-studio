//! Claude Code sessions, as the cc-tab hook writes them down: one
//! `<claude pid>.st` file per session in `%LOCALAPPDATA%\cc-tab` (state,
//! detail, topic, since, transcript, seq, split by U+001F). The island tells
//! when a session finishes, waits for an approval or fails, so nobody has to
//! go through the terminal tabs to find out.
//!
//! One thread sleeps on a change notification for that folder (and wakes
//! every few seconds to drop sessions whose process ended); it reads a few
//! small files only when something there changed.

use crate::events::{AppEvent, EventSink};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE, WAIT_FAILED};
use windows::Win32::Storage::FileSystem::{FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE};
use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION};

/// What happened to a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudeChange {
    /// A turn ended (`detail` is how long it took, "2:10").
    Done,
    /// It waits for a permission answer.
    Approval,
    /// The turn failed.
    Error,
    /// It works again (or stopped) after waiting for an approval: the
    /// approval notice can go.
    Resumed,
}

#[derive(Clone, Debug)]
pub struct ClaudeEvent {
    pub change: ClaudeChange,
    pub pid: u32,
    /// Claude Code's own title for the session, else its first prompt.
    pub name: String,
    /// "Origin" or "Loka" (which Claude Code config it runs under).
    pub config: &'static str,
    pub detail: String,
}

pub struct ClaudeService;

impl ClaudeService {
    pub fn start(sink: EventSink) -> Result<Self, String> {
        let dir = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("cc-tab")).ok_or("no LOCALAPPDATA")?;
        std::thread::Builder::new().name("claude".into()).spawn(move || run(&dir, &sink)).map_err(|e| e.to_string())?;
        Ok(Self)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Session {
    state: String,
    detail: String,
    topic: String,
    tx: String,
}

fn run(dir: &Path, sink: &EventSink) {
    // cc-tab may be set up after the island starts.
    while !dir.is_dir() {
        std::thread::sleep(Duration::from_secs(60));
    }
    let Ok(watch) = (unsafe { FindFirstChangeNotificationW(&HSTRING::from(dir.as_os_str()), false, FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME) }) else {
        crate::pwarn!("claude sessions: can't watch {}", dir.display());
        return;
    };
    crate::pinfo!("claude sessions: watching {}", dir.display());
    let mut known: HashMap<u32, Session> = scan(dir);
    loop {
        let r = unsafe { WaitForSingleObject(watch, 5000) };
        if r == WAIT_FAILED {
            break;
        }
        // A hook writes its file and the log together: let both land.
        std::thread::sleep(Duration::from_millis(120));
        unsafe {
            let _ = FindNextChangeNotification(watch);
        }
        let now = scan(dir);
        for (pid, s) in &now {
            let Some(before) = known.get(pid) else { continue };
            if before.state == s.state {
                continue;
            }
            let change = match s.state.as_str() {
                "done" if matches!(before.state.as_str(), "working" | "approval") => ClaudeChange::Done,
                "approval" => ClaudeChange::Approval,
                "error" => ClaudeChange::Error,
                _ if before.state == "approval" => ClaudeChange::Resumed,
                _ => continue,
            };
            sink.send(AppEvent::Claude(ClaudeEvent { change, pid: *pid, name: name_of(s), config: config_of(&s.tx), detail: s.detail.clone() }));
        }
        for pid in known.keys().filter(|p| !now.contains_key(p)) {
            if known[pid].state == "approval" {
                sink.send(AppEvent::Claude(ClaudeEvent { change: ClaudeChange::Resumed, pid: *pid, name: String::new(), config: "", detail: String::new() }));
            }
        }
        known = now;
    }
    unsafe {
        let _ = FindCloseChangeNotification(watch);
    }
}

/// Every live session's state (a file whose Claude has exited is skipped;
/// cc-tab leaves it behind).
fn scan(dir: &Path) -> HashMap<u32, Session> {
    let mut out = HashMap::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("st") {
            continue;
        }
        let Some(pid) = p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u32>().ok()) else { continue };
        if !alive(pid) {
            continue;
        }
        if let Some(s) = std::fs::read_to_string(&p).ok().and_then(|t| parse(&t)) {
            out.insert(pid, s);
        }
    }
    out
}

fn parse(text: &str) -> Option<Session> {
    let v: Vec<&str> = text.split('\u{1f}').collect();
    (v.len() == 6).then(|| Session { state: v[0].into(), detail: v[1].into(), topic: v[2].into(), tx: v[4].into() })
}

fn alive(pid: u32) -> bool {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(h);
        ok
    }
}

fn config_of(tx: &str) -> &'static str {
    if tx.to_ascii_lowercase().contains("\\.claude-loka\\") {
        "Loka"
    } else {
        "Origin"
    }
}

/// The session's title in its transcript (the last `customTitle` or
/// `aiTitle`), else the topic cc-tab kept from the first prompt.
fn name_of(s: &Session) -> String {
    transcript_title(&s.tx).filter(|t| !t.is_empty()).unwrap_or_else(|| if s.topic.is_empty() { "Claude".into() } else { s.topic.clone() })
}

fn transcript_title(tx: &str) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(tx).ok()?;
    let len = f.metadata().ok()?.len();
    let take = len.min(256 * 1024);
    f.seek(SeekFrom::Start(len - take)).ok()?;
    let mut buf = Vec::with_capacity(take as usize);
    f.read_to_end(&mut buf).ok()?;
    title_in(&String::from_utf8_lossy(&buf))
}

/// The newest of `"customTitle":"…"` / `"aiTitle":"…"` in `tail` (a name
/// given by hand wins over Claude's own when it is newer).
fn title_in(tail: &str) -> Option<String> {
    let at = |key: &str| tail.rfind(key).map(|i| (i, i + key.len()));
    let (_, start) = [at("\"customTitle\":\""), at("\"aiTitle\":\"")].into_iter().flatten().max_by_key(|(i, _)| *i)?;
    let mut out = String::new();
    let mut it = tail[start..].chars();
    while let Some(c) = it.next() {
        match c {
            '"' => return Some(out.trim().to_string()),
            '\\' => match it.next()? {
                'n' | 'r' | 't' => out.push(' '),
                'u' => {
                    let h: String = it.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                o => out.push(o),
            },
            c => out.push(c),
        }
    }
    None
}

/// Bring the Windows Terminal window with the session `name` in front (the
/// one whose title shows it: cc-tab puts the session's name in its tab's
/// title), else the terminal used last. UI thread, right after a click on
/// the island (Windows lets the app that took the last input do this).
pub fn focus_terminal(name: &str) {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowTextW, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE};
    struct Find<'a> {
        name: &'a str,
        first: HWND,
        named: HWND,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let f = &mut *(lp.0 as *mut Find);
        let mut buf = [0u16; 256];
        let n = GetClassNameW(h, &mut buf) as usize;
        if String::from_utf16_lossy(&buf[..n]) != "CASCADIA_HOSTING_WINDOW_CLASS" || !IsWindowVisible(h).as_bool() {
            return BOOL(1);
        }
        if f.first.is_invalid() {
            f.first = h;
        }
        let n = GetWindowTextW(h, &mut buf) as usize;
        if !f.name.is_empty() && String::from_utf16_lossy(&buf[..n]).contains(f.name) {
            f.named = h;
            return BOOL(0);
        }
        BOOL(1)
    }
    let short: String = name.chars().take(24).collect();
    let mut f = Find { name: short.trim_end_matches('…'), first: HWND::default(), named: HWND::default() };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut f as *mut Find as isize));
        let h = if f.named.is_invalid() { f.first } else { f.named };
        if h.is_invalid() {
            return;
        }
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetForegroundWindow(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_cc_tab_state_file() {
        let s = parse("done\u{1f}2:10\u{1f}fix the tab\u{1f}123\u{1f}C:\\Users\\k\\.claude-loka\\projects\\p\\a.jsonl\u{1f}99").unwrap();
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.topic.as_str()), ("done", "2:10", "fix the tab"));
        assert_eq!(config_of(&s.tx), "Loka");
        assert_eq!(config_of("C:\\Users\\k\\.claude\\projects\\p\\a.jsonl"), "Origin");
        assert!(parse("done\u{1f}x").is_none());
    }

    #[test]
    fn newest_title_wins() {
        let tail = r#"{"type":"x","aiTitle":"Old name"}
{"customTitle":"Mine \u00e9"}
"#;
        assert_eq!(title_in(tail).as_deref(), Some("Mine é"));
        assert_eq!(title_in(r#"{"aiTitle":"Island on both screens"}"#).as_deref(), Some("Island on both screens"));
        assert_eq!(title_in("nothing"), None);
    }
}
