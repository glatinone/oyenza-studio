//! A Teams call, without Teams' own API (the work tenant switches it off):
//! Windows notes when an app starts and stops using the microphone
//! (`CapabilityAccessManager\ConsentStore\microphone\<app>`:
//! `LastUsedTimeStop` is 0 while it uses it), and Teams keeps the
//! microphone open for the whole call, muted or not. One thread waits on
//! that key's change notification; nothing polls.
//!
//! Mute and leave are Teams' own shortcuts (Ctrl+Shift+M, Ctrl+Shift+H),
//! pressed in its call window after a click on the island.

use crate::events::{AppEvent, EventSink};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::System::Registry::{RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

const KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone\\MSTeams_8wekyb3d8bbwe");

pub struct CallsService;

impl CallsService {
    pub fn start(sink: EventSink) -> Result<Self, String> {
        std::thread::Builder::new().name("calls".into()).spawn(move || run(&sink)).map_err(|e| e.to_string())?;
        Ok(Self)
    }
}

fn run(sink: &EventSink) {
    unsafe {
        let mut key = HKEY::default();
        // Teams may never have used the microphone yet: look again now and then.
        while RegOpenKeyExW(HKEY_CURRENT_USER, KEY, 0, KEY_READ | KEY_NOTIFY, &mut key).is_err() {
            std::thread::sleep(std::time::Duration::from_secs(300));
        }
        let Ok(ev) = CreateEventW(None, false, false, None) else { return };
        let mut in_call = false;
        loop {
            let _ = RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, ev, true);
            let now = qword(key, w!("LastUsedTimeStart")).unwrap_or(0) != 0 && qword(key, w!("LastUsedTimeStop")) == Some(0);
            if now != in_call {
                in_call = now;
                crate::pinfo!("calls: Teams {}", if now { "call started" } else { "call ended" });
                sink.send(AppEvent::TeamsCall(now));
            }
            WaitForSingleObject(ev, 60_000);
        }
        #[allow(unreachable_code)]
        {
            let _ = RegCloseKey(key);
        }
    }
}

unsafe fn qword(key: HKEY, name: PCWSTR) -> Option<u64> {
    let mut v = 0u64;
    let mut size = 8u32;
    RegQueryValueExW(key, name, None, None, Some(&mut v as *mut u64 as *mut u8), Some(&mut size)).ok().ok()?;
    Some(v)
}

/// Teams' call window: one of its windows that isn't the main one (Chat,
/// Activity...), else the main one.
fn call_window() -> Option<HWND> {
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible};
    struct Found {
        call: Option<HWND>,
        main: Option<HWND>,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let f = &mut *(lp.0 as *mut Found);
        if !IsWindowVisible(h).as_bool() {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return BOOL(1) };
        let mut buf = [0u16; 512];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(p);
        if !ok || !String::from_utf16_lossy(&buf[..n as usize]).to_ascii_lowercase().ends_with("\\ms-teams.exe") {
            return BOOL(1);
        }
        let n = GetWindowTextW(h, &mut buf) as usize;
        let title = String::from_utf16_lossy(&buf[..n]);
        if title.is_empty() {
            return BOOL(1);
        }
        let main = ["Chat |", "Activity |", "Calendar |", "Teams and channels |", "Calls |", "OneDrive |", "Copilot |", "Settings |"].iter().any(|p| title.starts_with(p));
        if main {
            f.main.get_or_insert(h);
        } else if title.ends_with("Microsoft Teams") {
            f.call.get_or_insert(h);
        }
        BOOL(1)
    }
    let mut f = Found { call: None, main: None };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut f as *mut Found as isize));
    }
    f.call.or(f.main)
}

/// Bring Teams' call window in front (a click on the call's notice).
pub fn focus_call() {
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE};
    let Some(h) = call_window() else { return };
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetForegroundWindow(h);
    }
}

/// Press Ctrl+Shift+`key` in Teams' call window, then give the focus back
/// to the window that had it. UI thread, right after a click on the island.
pub fn teams_shortcut(key: u8) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_SHIFT};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};
    let Some(h) = call_window() else {
        crate::pinfo!("calls: no Teams window for the shortcut");
        return;
    };
    let key_input = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } },
    };
    let k = VIRTUAL_KEY(key as u16);
    let seq = [key_input(VK_CONTROL, false), key_input(VK_SHIFT, false), key_input(k, false), key_input(k, true), key_input(VK_SHIFT, true), key_input(VK_CONTROL, true)];
    unsafe {
        let before = GetForegroundWindow();
        if !SetForegroundWindow(h).as_bool() {
            crate::pinfo!("calls: Teams wouldn't come in front");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(60));
        SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
        std::thread::sleep(std::time::Duration::from_millis(60));
        if !before.is_invalid() && before != h {
            let _ = SetForegroundWindow(before);
        }
    }
}
