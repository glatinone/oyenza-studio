//! floaty-ask: Claude Code's `PermissionRequest` hook, answered on the island.
//!
//! Claude Code runs it with the request's JSON on stdin. It hands that to
//! Floaty on 127.0.0.1 (the port and the secret Floaty wrote in
//! `%APPDATA%\Poni\claude-ask.txt`), waits, and prints what Floaty sends
//! back: the hook's decision when Allow, Always or Deny was clicked, nothing
//! otherwise. Nothing printed means Claude Code asks in the terminal as
//! usual, so a missing, closed or busy Floaty never answers for anyone.
//!
//! Std only and a GUI-subsystem exe (no console flashes): it starts in a few
//! milliseconds. Claude Code ending the hook (an answer typed in the
//! terminal) closes the connection, and Floaty takes the card away.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::Duration;

fn main() {
    let mut req = Vec::new();
    if std::io::stdin().read_to_end(&mut req).is_err() || req.is_empty() {
        return;
    }
    if let Some(out) = ask(&req) {
        let mut so = std::io::stdout();
        let _ = so.write_all(&out);
        let _ = so.flush();
    }
}

fn ask(req: &[u8]) -> Option<Vec<u8>> {
    let dir = std::env::var_os("APPDATA")?;
    let file = std::path::Path::new(&dir).join("Poni").join("claude-ask.txt");
    let text = std::fs::read_to_string(file).ok()?;
    let mut it = text.split_whitespace();
    let port: u16 = it.next()?.parse().ok()?;
    let token = it.next()?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
    let _ = s.set_nodelay(true);
    // "<secret> <claude pid> <bytes> <attended>\n" then the hook's JSON as
    // it came. Claude Code tells its hooks its pid and whether someone sits
    // at the session (0 for `claude -p`).
    let pid = std::env::var("CLAUDE_PID").ok().and_then(|p| p.trim().parse::<u32>().ok()).unwrap_or_else(claude_pid);
    let attended = if std::env::var("CLAUDE_CODE_SESSION_ATTENDED").is_ok_and(|v| v.trim() == "0") { 0 } else { 1 };
    let head = format!("{token} {pid} {} {attended}\n", req.len());
    s.write_all(head.as_bytes()).ok()?;
    s.write_all(req).ok()?;
    s.flush().ok()?;
    // The answer, then Floaty closes. No shutdown of our side: Floaty reads
    // it to learn that this hook has ended (Claude Code killed it).
    let mut out = Vec::new();
    s.read_to_end(&mut out).ok()?;
    let _ = s.shutdown(Shutdown::Both);
    let out = out.trim_ascii().to_vec();
    (!out.is_empty()).then_some(out)
}

/// The claude.exe this hook runs under (claude.exe -> [shell] -> this), so
/// Floaty can name the session the way cc-tab does; 0 when not found.
#[cfg(windows)]
fn claude_pid() -> u32 {
    use std::ffi::c_void;
    #[repr(C)]
    struct Pe {
        size: u32,
        usage: u32,
        pid: u32,
        heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        prio: i32,
        flags: u32,
        exe: [u16; 260],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(s: *mut c_void, e: *mut Pe) -> i32;
        fn Process32NextW(s: *mut c_void, e: *mut Pe) -> i32;
        fn CloseHandle(h: *mut c_void) -> i32;
    }
    let mut parent = std::collections::HashMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(2, 0);
        if snap.is_null() || snap as isize == -1 {
            return 0;
        }
        let mut e: Pe = std::mem::zeroed();
        e.size = std::mem::size_of::<Pe>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let l = e.exe.iter().position(|&c| c == 0).unwrap_or(260);
            parent.insert(e.pid, (e.parent, String::from_utf16_lossy(&e.exe[..l]).to_ascii_lowercase()));
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    let mut p = std::process::id();
    for _ in 0..16 {
        let Some(&(pp, _)) = parent.get(&p) else { break };
        p = pp;
        if parent.get(&p).is_some_and(|x| x.1 == "claude.exe") {
            return p;
        }
    }
    0
}

#[cfg(not(windows))]
fn claude_pid() -> u32 {
    0
}
