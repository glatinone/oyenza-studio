//! "Every display": the island shows on each screen at once. The island
//! itself lives on the screen under the pointer (where it can be used);
//! every other screen shows a live copy of it, a DWM thumbnail of the
//! renderer's window. Windows composes the copies itself, so they draw
//! nothing here and cost next to no CPU; they take no input.

use crate::window::{self, MonitorInfo};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const MIRROR_CLASS: PCWSTR = w!("Poni.Mirror");

unsafe extern "system" fn mirror_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        // Placed and sized from the island; a display change must not resize it.
        WM_DPICHANGED => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn register_class() -> bool {
    static ONCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ONCE.get_or_init(|| unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else { return false };
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(mirror_wndproc),
            hInstance: hinst.into(),
            lpszClassName: MIRROR_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc) != 0
    })
}

/// One screen's copy.
struct Mirror {
    monitor: isize,
    hwnd: HWND,
    thumb: isize,
    /// The window it copies and that window's size when it was taken: a
    /// copy kept across the source being resized (moving to a screen with
    /// another scale) turned into a black box, so it is taken again.
    source: (isize, i32, i32),
    /// Where it was last put (screen pixels) and whether it showed.
    at: RECT,
    shown: bool,
}

impl Drop for Mirror {
    fn drop(&mut self) {
        unsafe {
            let _ = DwmUnregisterThumbnail(self.thumb);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

#[derive(Default)]
pub struct Mirrors {
    list: Vec<Mirror>,
    /// Syncs since the copies were last put back on top.
    ticks: u32,
}

impl Mirrors {
    /// Drop every copy (the island is back on one screen).
    pub fn clear(&mut self) {
        self.list.clear();
    }

    /// Show a copy of `source` (the renderer's window, on `on`) on every
    /// other screen, placed where the island hangs on its own screen and
    /// scaled for that screen's DPI. Hidden while the source is, and on a
    /// screen where `covered` (a full-screen app fills it).
    pub fn sync(&mut self, source: Option<HWND>, on: &MonitorInfo, covered: impl Fn(&RECT) -> bool) {
        let Some(source) = source else { return self.clear() };
        let others: Vec<MonitorInfo> = monitors().into_iter().filter(|m| m.handle != on.handle).collect();
        self.list.retain(|m| others.iter().any(|o| o.handle == m.monitor));
        let mut src = RECT::default();
        let visible = unsafe { IsWindowVisible(source).as_bool() && GetWindowRect(source, &mut src).is_ok() };
        self.ticks += 1;
        let raise = self.ticks >= 8;
        if raise {
            self.ticks = 0;
        }
        for t in &others {
            let i = match self.list.iter().position(|m| m.monitor == t.handle) {
                Some(i) => i,
                None => match create(source, t.handle) {
                    Some(m) => {
                        self.list.push(m);
                        self.list.len() - 1
                    }
                    None => continue,
                },
            };
            let size = (source.0 as isize, src.right - src.left, src.bottom - src.top);
            if self.list[i].source != size && visible {
                let m = &mut self.list[i];
                unsafe {
                    let _ = DwmUnregisterThumbnail(m.thumb);
                    match DwmRegisterThumbnail(m.hwnd, source) {
                        Ok(t) => m.thumb = t,
                        Err(e) => crate::pwarn!("every display: no copy of the island ({e})"),
                    }
                }
                m.source = size;
                // Its place and size are set again below.
                m.shown = false;
            }
            let m = &mut self.list[i];
            let at = place(&src, on, t);
            let show = visible && at.right > at.left && !covered(&t.rect);
            if show != m.shown || (show && at != m.at) {
                let size_changed = (at.right - at.left, at.bottom - at.top) != (m.at.right - m.at.left, m.at.bottom - m.at.top);
                unsafe {
                    if show {
                        if size_changed || !m.shown {
                            let p = DWM_THUMBNAIL_PROPERTIES {
                                dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_OPACITY | DWM_TNP_VISIBLE | DWM_TNP_SOURCECLIENTAREAONLY,
                                rcDestination: RECT { left: 0, top: 0, right: at.right - at.left, bottom: at.bottom - at.top },
                                rcSource: RECT::default(),
                                opacity: 255,
                                fVisible: BOOL(1),
                                fSourceClientAreaOnly: BOOL(0),
                            };
                            let _ = DwmUpdateThumbnailProperties(m.thumb, &p);
                        }
                        let _ = SetWindowPos(
                            m.hwnd,
                            HWND_TOPMOST,
                            at.left,
                            at.top,
                            at.right - at.left,
                            at.bottom - at.top,
                            SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        );
                    } else {
                        let _ = ShowWindow(m.hwnd, SW_HIDE);
                    }
                }
                m.at = at;
                m.shown = show;
            } else if show && raise {
                // Something else top-most may have come over it.
                unsafe {
                    let _ = SetWindowPos(m.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
                }
            }
        }
    }
}

fn create(source: HWND, monitor: isize) -> Option<Mirror> {
    if !register_class() {
        return None;
    }
    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            MIRROR_CLASS,
            w!("Poni"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            GetModuleHandleW(None).ok()?,
            None,
        )
        .ok()?;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        match DwmRegisterThumbnail(hwnd, source) {
            Ok(thumb) => Some(Mirror { monitor, hwnd, thumb, source: (0, 0, 0), at: RECT::default(), shown: false }),
            Err(e) => {
                crate::pwarn!("every display: no copy of the island ({e})");
                let _ = DestroyWindow(hwnd);
                None
            }
        }
    }
}

/// Where the copy goes on `to`: the same share of the way across, the same
/// distance below where the island hangs, scaled by the two screens' DPI.
fn place(src: &RECT, from: &MonitorInfo, to: &MonitorInfo) -> RECT {
    let s = to.dpi as f32 / from.dpi.max(1) as f32;
    let w = ((src.right - src.left) as f32 * s).round() as i32;
    let h = ((src.bottom - src.top) as f32 * s).round() as i32;
    let fw = (from.rect.right - from.rect.left).max(1) as f32;
    let tw = to.rect.right - to.rect.left;
    let share = (((src.left + src.right) as f32 / 2.0 - from.rect.left as f32) / fw).clamp(0.0, 1.0);
    let cx = to.rect.left as f32 + share * tw as f32;
    let left = ((cx - w as f32 / 2.0).round() as i32).clamp(to.rect.left, (to.rect.right - w).max(to.rect.left));
    let top = to.top() + ((src.top - from.top()) as f32 * s).round() as i32;
    RECT { left, top, right: left + w, bottom: top + h }
}

/// Every screen, as the island sees it.
pub fn monitors() -> Vec<MonitorInfo> {
    unsafe extern "system" fn each(h: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
        let v = &mut *(out.0 as *mut Vec<MonitorInfo>);
        if let Some(m) = window::monitor_info(h) {
            v.push(m);
        }
        BOOL(1)
    }
    let mut v: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(HDC::default(), None, Some(each), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(left: i32, w: i32, h: i32, dpi: u32) -> MonitorInfo {
        let rect = RECT { left, top: 0, right: left + w, bottom: h };
        MonitorInfo { handle: left as isize, rect, work: rect, dpi }
    }

    #[test]
    fn copy_keeps_its_share_of_the_way_across() {
        let a = mon(0, 1920, 1200, 96);
        let b = mon(-1920, 1920, 1080, 96);
        let src = RECT { left: 614, top: 0, right: 1306, bottom: 369 };
        assert_eq!(place(&src, &a, &b), RECT { left: -1306, top: 0, right: -614, bottom: 369 });
    }

    #[test]
    fn copy_scales_and_stays_on_its_screen() {
        let a = mon(0, 1920, 1080, 96);
        let b = mon(1920, 2880, 1800, 192);
        let src = RECT { left: 0, top: 0, right: 692, bottom: 369 };
        let r = place(&src, &a, &b);
        assert_eq!((r.right - r.left, r.bottom - r.top), (1384, 738));
        assert_eq!(r.left, 1920);
    }
}
