//! The ping overlay shown over a game: two lines in a corner of the screen,
//! the router's and the internet's reply time, and whose side a spike is on.
//!
//! A window of its own, drawn with GDI on its own thread like the tray icon,
//! because the egui window is hidden while a game runs and a hidden egui
//! window gets no frames. It is a separate, click-through, topmost window:
//! nothing is injected into the game, which is what anti-cheat looks for.
//! Over a game in exclusive fullscreen Windows may not show it at all; over
//! borderless windowed it shows.

use crate::game::{Blame, Reading};
use crate::i18n;
use crate::monitor::{Seen, Snapshot, Status};
use crate::settings::Settings;

/// Above this much traffic through the adapter, something besides the game is
/// using the link: a match needs well under one Mbit/s.
const BUSY_MBPS: f64 = 2.0;
/// Sweeps kept for [`crate::game::blame`]: ten seconds at the game cadence.
const WINDOW: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Bad,
    Dim,
}

/// The router's and the fastest internet target's reply in a snapshot, as
/// the overlay's history keeps them.
pub fn reading(snap: &Snapshot) -> Reading {
    let rtt = |key: &str| snap.results.get(key).filter(|s| s.ok).and_then(|s| s.rtt_ms);
    let internet = ["cloudflare", "google"].iter().filter_map(|k| rtt(k)).reduce(f64::min);
    Reading { router: rtt("gateway"), internet }
}

/// What the overlay says: two lines and their tones.
pub fn lines(
    seen: Seen<'_>,
    now: Option<Reading>,
    blame: Blame,
    busy_mbps: Option<f64>,
    s: &Settings,
) -> [(String, Tone); 2] {
    let status = match seen {
        Seen::Verdict(status, _) => status,
        _ => {
            return [(i18n::ov_not_measuring().to_string(), Tone::Dim), (String::new(), Tone::Dim)]
        }
    };
    let ms = |v: Option<f64>| v.map_or_else(|| "—".to_string(), |v| format!("{v:.0} ms"));
    let r = now.unwrap_or(Reading { router: None, internet: None });
    let tone = match r.internet {
        Some(v) if v <= s.ping_ok_ms => Tone::Good,
        Some(v) if v <= s.ping_bad_ms => Tone::Warn,
        Some(_) => Tone::Bad,
        None => Tone::Dim,
    };
    let first = (i18n::ov_ping(&ms(r.router), &ms(r.internet)), tone);

    let second = match status {
        Status::IspDown | Status::LanDown | Status::AdapterDown | Status::DnsFail => {
            (status.headline().to_string(), Tone::Bad)
        }
        _ => match (blame, busy_mbps.filter(|m| *m > BUSY_MBPS)) {
            (Blame::Local, _) => (i18n::ov_local().to_string(), Tone::Bad),
            (Blame::Beyond, _) => (i18n::ov_beyond().to_string(), Tone::Warn),
            (_, Some(m)) => (i18n::ov_busy(m), Tone::Warn),
            (Blame::Clean, None) => (i18n::ov_clean().to_string(), Tone::Good),
            (Blame::Unknown, None) => (i18n::ov_unknown().to_string(), Tone::Dim),
        },
    };
    [first, second]
}

#[cfg(windows)]
pub use imp::Overlay;

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        GetLastError, COLORREF, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, RECT,
        WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
        InvalidateRect, SelectObject, SetBkMode, SetTextColor, TextOutW, HFONT, PAINTSTRUCT,
        TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        PostMessageW, PostQuitMessage, RegisterClassW, SetLayeredWindowAttributes, SetTimer,
        ShowWindow, TranslateMessage, LWA_ALPHA, MSG, SW_HIDE, SW_SHOWNOACTIVATE, WM_CLOSE,
        WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    use super::{lines, reading, Tone, WINDOW};
    use crate::game::Reading;
    use crate::monitor::{Seen, Shared};
    use crate::probe::netstate::{self, LinkCounters};

    const CLASS: &str = "NetDoctorOverlay";
    const TIMER_ID: usize = 1;
    const W: i32 = 330;
    const H: i32 = 48;
    /// From the top-left corner of the primary screen.
    const AT: i32 = 12;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
        COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
    }

    /// The window's palette, as the rest of the app.
    fn colour(t: Tone) -> COLORREF {
        match t {
            Tone::Good => rgb(0x3d, 0xdc, 0x84),
            Tone::Warn => rgb(0xff, 0xc4, 0x4d),
            Tone::Bad => rgb(0xff, 0x5f, 0x5f),
            Tone::Dim => rgb(0x9a, 0xa3, 0xb4),
        }
    }

    /// The overlay, owned by the app. Dropping it closes the window.
    pub struct Overlay {
        hwnd: isize,
        thread: Option<JoinHandle<()>>,
    }

    impl Overlay {
        /// `None` when the window could not be made; the app runs without it.
        pub fn start(shared: Arc<Shared>) -> Option<Overlay> {
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Option<isize>>();
            let thread = std::thread::Builder::new()
                .name("netdoctor-overlay".into())
                .spawn(move || {
                    let hwnd = unsafe { create(shared) };
                    let _ = ready_tx.send(hwnd.map(|h| h.0 as isize));
                    if hwnd.is_some() {
                        unsafe { pump() };
                    }
                })
                .ok()?;
            match ready_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(Some(hwnd)) => Some(Overlay { hwnd, thread: Some(thread) }),
                _ => None,
            }
        }
    }

    impl Drop for Overlay {
        fn drop(&mut self) {
            unsafe {
                let _ = PostMessageW(HWND(self.hwnd as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    struct State {
        hwnd: HWND,
        shared: Arc<Shared>,
        font: HFONT,
        shown: bool,
        history: VecDeque<Reading>,
        last_ts: f64,
        counters: Option<(LinkCounters, Instant)>,
        busy: Option<f64>,
        text: [(String, Tone); 2],
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    fn with_state(f: impl FnOnce(&mut State)) {
        STATE.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                if let Some(state) = guard.as_mut() {
                    f(state);
                }
            }
        });
    }

    impl State {
        fn tick(&mut self) {
            let settings = self.shared.settings.lock().unwrap_or_else(|p| p.into_inner()).clone();
            let want = self.shared.gaming.load(Ordering::Relaxed) && settings.game_overlay;
            if !want {
                if self.shown {
                    unsafe {
                        let _ = ShowWindow(self.hwnd, SW_HIDE);
                    }
                    self.shown = false;
                }
                self.history.clear();
                self.counters = None;
                return;
            }

            let last = self.shared.last.lock().unwrap_or_else(|p| p.into_inner()).clone();
            let interval = settings.interval();
            let seen = Seen::of(&last, !self.shared.paused(), crate::store::now(), interval);
            let now_reading = matches!(seen, Seen::Verdict(..)).then(|| reading(&last));
            if let Some(r) = now_reading {
                if last.ts != self.last_ts {
                    self.last_ts = last.ts;
                    self.history.push_back(r);
                    while self.history.len() > WINDOW {
                        self.history.pop_front();
                    }
                }
            }

            // Traffic over the last few seconds, not per tick: one burst of a
            // page loading in the background is not "something is
            // downloading".
            if let Some(now) = netstate::link_counters(last.net.if_index) {
                match self.counters {
                    Some((before, at)) if at.elapsed() >= Duration::from_secs(3) => {
                        self.busy = now.mbps_since(&before, at.elapsed().as_secs_f64());
                        self.counters = Some((now, Instant::now()));
                    }
                    None => self.counters = Some((now, Instant::now())),
                    _ => {}
                }
            }

            let window: Vec<Reading> = self.history.iter().copied().collect();
            let text = lines(seen, now_reading, crate::game::blame(&window), self.busy, &settings);
            if text != self.text || !self.shown {
                self.text = text;
                unsafe {
                    let _ = InvalidateRect(self.hwnd, None, true);
                }
            }
            if !self.shown {
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                }
                self.shown = true;
            }
        }

        fn paint(&self) {
            unsafe {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(self.hwnd, &mut ps);
                let bg = CreateSolidBrush(rgb(0x14, 0x16, 0x1a));
                let rect = RECT { left: 0, top: 0, right: W, bottom: H };
                FillRect(hdc, &rect, bg);
                let _ = DeleteObject(bg);
                let old = SelectObject(hdc, self.font);
                SetBkMode(hdc, TRANSPARENT);
                for (i, (text, tone)) in self.text.iter().enumerate() {
                    SetTextColor(hdc, colour(*tone));
                    let units: Vec<u16> = text.encode_utf16().collect();
                    let _ = TextOutW(hdc, 10, 5 + i as i32 * 20, &units);
                }
                SelectObject(hdc, old);
                let _ = EndPaint(self.hwnd, &ps);
            }
        }
    }

    unsafe fn create(shared: Arc<Shared>) -> Option<HWND> {
        let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }.ok()?.into();
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&wc) } == 0
            && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
        {
            return None;
        }
        // Click-through (layered + transparent), never activated, never on
        // the taskbar, always on top. The game keeps the keyboard and mouse.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                AT,
                AT,
                W,
                H,
                None,
                None,
                hinstance,
                None,
            )
        }
        .ok()?;
        unsafe {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 215, LWA_ALPHA);
        }
        let face = wide("Segoe UI");
        let font = unsafe {
            CreateFontW(-15, 0, 0, 0, 600, 0, 0, 0, 0, 0, 0, 5, 0, PCWSTR(face.as_ptr()))
        };
        let state = State {
            hwnd,
            shared,
            font,
            shown: false,
            history: VecDeque::new(),
            last_ts: 0.0,
            counters: None,
            busy: None,
            text: [(String::new(), Tone::Dim), (String::new(), Tone::Dim)],
        };
        STATE.with(|cell| *cell.borrow_mut() = Some(state));
        unsafe { SetTimer(hwnd, TIMER_ID, 500, None) };
        Some(hwnd)
    }

    unsafe fn pump() {
        let mut msg = MSG::default();
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().take() {
                unsafe {
                    let _ = DeleteObject(state.font);
                }
            }
        });
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_TIMER => with_state(State::tick),
            WM_PAINT => with_state(|s| s.paint()),
            WM_CLOSE => unsafe {
                let _ = DestroyWindow(hwnd);
            },
            WM_DESTROY => unsafe { PostQuitMessage(0) },
            _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
        LRESULT(0)
    }
}

/// Without Windows there is no overlay.
#[cfg(not(windows))]
pub struct Overlay;

#[cfg(not(windows))]
impl Overlay {
    pub fn start(_shared: std::sync::Arc<crate::monitor::Shared>) -> Option<Overlay> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(router: f64, internet: f64) -> Option<Reading> {
        Some(Reading { router: Some(router), internet: Some(internet) })
    }

    #[test]
    fn nothing_measured_says_so_instead_of_a_ping() {
        let s = Settings::default();
        let [first, second] = lines(Seen::Paused, at(3.0, 20.0), Blame::Clean, None, &s);
        assert_eq!(first.1, Tone::Dim);
        assert!(second.0.is_empty());
    }

    #[test]
    fn a_spike_on_this_side_outranks_everything_short_of_an_outage() {
        let s = Settings::default();
        let ok = Seen::Verdict(Status::Ok, "");
        let [first, second] = lines(ok, at(3.0, 22.0), Blame::Local, Some(40.0), &s);
        assert_eq!(first.1, Tone::Good);
        assert_eq!(second.1, Tone::Bad);
        // Busy traffic is named when no spike is.
        let [_, busy] = lines(ok, at(3.0, 22.0), Blame::Clean, Some(12.0), &s);
        assert_eq!(busy.1, Tone::Warn);
        // A trickle is not "busy".
        let [_, calm] = lines(ok, at(3.0, 22.0), Blame::Clean, Some(0.3), &s);
        assert_eq!(calm.1, Tone::Good);
        // An outage is said as one.
        let down = Seen::Verdict(Status::IspDown, "");
        let [_, out] = lines(down, None, Blame::Beyond, None, &s);
        assert_eq!((out.0.as_str(), out.1), (Status::IspDown.headline(), Tone::Bad));
    }
}
