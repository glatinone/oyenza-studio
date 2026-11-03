//! The next meetings from classic Outlook's calendar, for a heads-up on the
//! island before each one starts (and its Teams link on a click).
//!
//! Outlook is only asked while it already runs (`GetActiveObject`: it is
//! never started from here), through its COM object model, late bound
//! (IDispatch), on a thread of its own: every `EVERY` and right away at
//! start. Nothing is written down; titles only go to the UI thread.

use crate::events::{AppEvent, EventSink};
use std::time::Duration;
use windows::core::{w, Interface, BSTR, GUID, HSTRING, PCWSTR, VARIANT};
use windows::Win32::System::Com::{CLSIDFromProgID, CoInitializeEx, IDispatch, COINIT_APARTMENTTHREADED, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS};
use windows::Win32::System::Ole::{GetActiveObject, DISPID_PROPERTYPUT};

/// Outlook's calendar is read again this often.
const EVERY: Duration = Duration::from_secs(5 * 60);
/// How far ahead it looks.
const AHEAD_S: i64 = 36 * 3600;

#[derive(Clone, Debug, PartialEq)]
pub struct Meeting {
    pub title: String,
    pub start: i64,
    pub end: i64,
    /// The Teams (or other https) link to join it, from its body or place.
    pub join: Option<String>,
}

pub struct MeetingsService;

impl MeetingsService {
    pub fn start(sink: EventSink) -> Result<Self, String> {
        std::thread::Builder::new().name("meetings".into()).spawn(move || run(&sink)).map_err(|e| e.to_string())?;
        Ok(Self)
    }
}

fn run(sink: &EventSink) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let mut last: Option<Vec<Meeting>> = None;
    let mut told_missing = false;
    loop {
        let now = crate::clock::unix_now();
        match unsafe { read_calendar(now) } {
            Ok(list) => {
                told_missing = false;
                if last.as_ref() != Some(&list) {
                    let next = list.first().map(|m| format!(", next in {} min", (m.start - now) / 60)).unwrap_or_default();
                    crate::pinfo!("meetings: {} in the next {} h{next}", list.len(), AHEAD_S / 3600);
                    sink.send(AppEvent::Meetings(list.clone()));
                    last = Some(list);
                }
            }
            Err(e) => {
                if !told_missing {
                    crate::pinfo!("meetings: Outlook's calendar not read ({e})");
                    told_missing = true;
                }
            }
        }
        std::thread::sleep(EVERY);
    }
}

unsafe fn read_calendar(now: i64) -> windows::core::Result<Vec<Meeting>> {
    let clsid = CLSIDFromProgID(w!("Outlook.Application"))?;
    let mut unk = None;
    GetActiveObject(&clsid, None, &mut unk)?;
    let app: IDispatch = unk.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?.cast()?;
    let ns = obj(&call(&app, "GetNamespace", &[VARIANT::from("MAPI")])?)?;
    // 9 = olFolderCalendar.
    let folder = obj(&call(&ns, "GetDefaultFolder", &[VARIANT::from(9i32)])?)?;
    let items = obj(&get(&folder, "Items")?)?;
    put(&items, "IncludeRecurrences", VARIANT::from(true))?;
    call(&items, "Sort", &[VARIANT::from("[Start]")])?;
    // DASL compares in UTC, whatever the PC's date format is.
    let (from, to) = (utc_text(now - 3600), utc_text(now + AHEAD_S));
    let filter = format!("@SQL=\"urn:schemas:calendar:dtstart\" < '{to}' AND \"urn:schemas:calendar:dtend\" > '{from}'");
    let found = obj(&call(&items, "Restrict", &[VARIANT::from(filter.as_str())])?)?;
    let mut out = Vec::new();
    let mut item = call(&found, "GetFirst", &[])?;
    // Recurrences make Count meaningless; the walk is bounded instead.
    for _ in 0..60 {
        let Ok(it) = obj(&item) else { break };
        if let Some(m) = meeting(&it) {
            if m.end > now && m.start < now + AHEAD_S {
                out.push(m);
            }
        }
        item = call(&found, "GetNext", &[])?;
    }
    out.sort_by_key(|m| m.start);
    Ok(out)
}

/// One calendar item, unless it's all day, cancelled, declined or free time.
unsafe fn meeting(it: &IDispatch) -> Option<Meeting> {
    let int = |name: &str| get(it, name).ok().and_then(|v| i32::try_from(&v).ok()).unwrap_or(0);
    if get(it, "AllDayEvent").ok().and_then(|v| bool::try_from(&v).ok()).unwrap_or(false) {
        return None;
    }
    // MeetingStatus 5/7: cancelled; ResponseStatus 4: declined; BusyStatus 0: free.
    if matches!(int("MeetingStatus"), 5 | 7) || int("ResponseStatus") == 4 || int("BusyStatus") == 0 {
        return None;
    }
    let start = unix_of(&get(it, "Start").ok()?)?;
    let end = unix_of(&get(it, "End").ok()?)?;
    let text = |name: &str| get(it, name).ok().and_then(|v| BSTR::try_from(&v).ok()).map(|b| b.to_string()).unwrap_or_default();
    let title = text("Subject");
    let join = join_link(&text("Location")).or_else(|| join_link(&text("Body")));
    Some(Meeting { title: if title.trim().is_empty() { "Meeting".into() } else { title.trim().to_string() }, start, end, join })
}

/// The first Teams/Zoom/Meet link in `text`.
pub fn join_link(text: &str) -> Option<String> {
    for prefix in ["https://teams.microsoft.com/", "https://teams.live.com/", "https://meet.google.com/", "https://zoom.us/", "https://us02web.zoom.us/", "https://us06web.zoom.us/"] {
        if let Some(i) = text.find(prefix) {
            let rest = &text[i..];
            let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | ')' | ']')).unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        }
    }
    None
}

/// An OLE date (local time, days since 1899-12-30) as Unix seconds.
unsafe fn unix_of(v: &VARIANT) -> Option<i64> {
    let raw = v.as_raw();
    // 7 = VT_DATE.
    if raw.Anonymous.Anonymous.vt != 7 {
        return None;
    }
    let days = raw.Anonymous.Anonymous.Anonymous.date;
    let local = ((days - 25569.0) * 86400.0).round() as i64;
    Some(local - utc_offset_s())
}

/// How far the PC's clock is ahead of UTC now, seconds.
fn utc_offset_s() -> i64 {
    let l = crate::clock::local_now();
    let local = poni_core::datetime::days_from_civil(l.year, l.month, l.day) * 86400 + l.hour as i64 * 3600 + l.minute as i64 * 60 + l.second as i64;
    ((local - crate::clock::unix_now()) as f64 / 60.0).round() as i64 * 60
}

/// `unix` as DASL wants it: "YYYY-MM-DD HH:MM" in UTC.
fn utc_text(unix: i64) -> String {
    let days = unix.div_euclid(86400);
    let secs = unix.rem_euclid(86400);
    let (y, m, d) = poni_core::datetime::civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

/// The object in `v` (VT_DISPATCH, which windows-core's own conversion
/// doesn't take, or VT_UNKNOWN).
fn obj(v: &VARIANT) -> windows::core::Result<IDispatch> {
    unsafe {
        let raw = v.as_raw();
        // 9 = VT_DISPATCH, 13 = VT_UNKNOWN: both hold the pointer in the same place.
        let p = raw.Anonymous.Anonymous.Anonymous.punkVal;
        if !matches!(raw.Anonymous.Anonymous.vt, 9 | 13) || p.is_null() {
            return Err(windows::Win32::Foundation::DISP_E_TYPEMISMATCH.into());
        }
        let unk: &windows::core::IUnknown = std::mem::transmute(&p);
        unk.cast()
    }
}

unsafe fn dispid(d: &IDispatch, name: &str) -> windows::core::Result<i32> {
    let h = HSTRING::from(name);
    let names = [PCWSTR(h.as_ptr())];
    let mut id = 0;
    d.GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, 0, &mut id)?;
    Ok(id)
}

unsafe fn invoke(d: &IDispatch, name: &str, flags: DISPATCH_FLAGS, args: &[VARIANT]) -> windows::core::Result<VARIANT> {
    let id = dispid(d, name)?;
    // IDispatch takes its arguments last to first.
    let mut rev: Vec<VARIANT> = args.iter().rev().cloned().collect();
    let mut put_id = DISPID_PROPERTYPUT;
    let params = DISPPARAMS {
        rgvarg: if rev.is_empty() { std::ptr::null_mut() } else { rev.as_mut_ptr() },
        rgdispidNamedArgs: if flags == DISPATCH_PROPERTYPUT { &mut put_id } else { std::ptr::null_mut() },
        cArgs: rev.len() as u32,
        cNamedArgs: u32::from(flags == DISPATCH_PROPERTYPUT),
    };
    let mut out = VARIANT::default();
    d.Invoke(id, &GUID::zeroed(), 0, flags, &params, Some(&mut out), None, None)?;
    Ok(out)
}

unsafe fn call(d: &IDispatch, name: &str, args: &[VARIANT]) -> windows::core::Result<VARIANT> {
    invoke(d, name, DISPATCH_METHOD | DISPATCH_PROPERTYGET, args)
}

unsafe fn get(d: &IDispatch, name: &str) -> windows::core::Result<VARIANT> {
    invoke(d, name, DISPATCH_PROPERTYGET, &[])
}

unsafe fn put(d: &IDispatch, name: &str, v: VARIANT) -> windows::core::Result<()> {
    invoke(d, name, DISPATCH_PROPERTYPUT, &[v]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_join_link() {
        let body = "Join: <https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=x> Meeting ID";
        assert_eq!(join_link(body).as_deref(), Some("https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=x"));
        assert_eq!(join_link("Room 4"), None);
    }

    #[test]
    fn dasl_time_is_utc_text() {
        assert_eq!(utc_text(0), "1970-01-01 00:00");
        assert_eq!(utc_text(1_762_156_800 + 3600 + 120), "2025-11-03 09:02");
    }
}
