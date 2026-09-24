//! The ping overlay shown over a game: in a corner of the screen, the
//! router's and the internet's reply time and whose side a spike is on, or
//! less when the player picks less.
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
use crate::settings::{Corner, OverlayContent, Settings};

/// Above this much traffic through the adapter, something besides the game is
/// using the link: a match needs well under one Mbit/s. It is named only
/// beside a spike past the router, where it may be the cause; traffic that
/// leaves the ping alone is no problem, and a call moves this much all game.
const BUSY_MBPS: f64 = 2.0;
/// Distance from the screen's edges.
const MARGIN: i32 = 12;

/// Whether the overlay is up: while a game runs, never over anything else.
/// Over every app was tried and dropped: over a browser it only got in the way.
pub fn wanted(s: &Settings, gaming: bool) -> bool {
    s.game_overlay && gaming
}

/// Where the overlay's top-left corner goes on a screen given as `(left,
/// top, right, bottom)`, for a window of `size`.
pub fn origin(screen: (i32, i32, i32, i32), corner: Corner, size: (i32, i32)) -> (i32, i32) {
    let (l, t, r, b) = screen;
    let (w, h) = size;
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => l + MARGIN,
        Corner::TopRight | Corner::BottomRight => r - MARGIN - w,
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => t + MARGIN,
        Corner::BottomLeft | Corner::BottomRight => b - MARGIN - h,
    };
    (x, y)
}

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

/// What the overlay shows, before it is drawn: the readings row and the
/// verdict row, each `None` when it is not shown.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// The router's reply. Drawn in the plain text colour: it is a reading,
    /// not a judgement.
    pub router: Option<String>,
    /// The internet's reply, in the tone the ping thresholds give it.
    pub internet: Option<(String, Tone)>,
    /// The verdict, an outage, or "not measuring".
    pub status: Option<(String, Tone)>,
}

impl View {
    /// The colour of the bar down the left edge: the verdict's, else the
    /// ping's. One glance at it says whether to read the rest.
    pub fn accent(&self) -> Tone {
        self.status.as_ref().or(self.internet.as_ref()).map_or(Tone::Dim, |(_, t)| *t)
    }

    /// How many rows it draws.
    pub fn rows(&self) -> usize {
        usize::from(self.router.is_some() || self.internet.is_some())
            + usize::from(self.status.is_some())
    }
}

/// What the overlay says, as much of it as [`Settings::overlay_content`]
/// asks for. The compact choices drop the verdict, never an outage: while
/// something is down, that is what it shows.
pub fn view(
    seen: Seen<'_>,
    now: Option<Reading>,
    blame: Blame,
    busy_mbps: Option<f64>,
    s: &Settings,
) -> View {
    let status = match seen {
        Seen::Verdict(status, _) => status,
        _ => {
            return View {
                router: None,
                internet: None,
                status: Some((i18n::ov_not_measuring().to_string(), Tone::Dim)),
            }
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
    let router = (s.overlay_content != OverlayContent::Internet).then(|| ms(r.router));
    let internet = Some((ms(r.internet), tone));
    let outage =
        matches!(status, Status::IspDown | Status::LanDown | Status::AdapterDown | Status::DnsFail);
    if s.overlay_content != OverlayContent::Full {
        return if outage {
            View {
                router: None,
                internet: None,
                status: Some((status.headline().to_string(), Tone::Bad)),
            }
        } else {
            View { router, internet, status: None }
        };
    }

    // The verdict row only when there is something to say: calm is what the
    // green bar already says, and a card that talks all game is not read.
    let verdict = if outage {
        Some((status.headline().to_string(), Tone::Bad))
    } else {
        match (blame, busy_mbps.filter(|m| *m > BUSY_MBPS)) {
            (Blame::Local, _) => Some((i18n::ov_local().to_string(), Tone::Bad)),
            // A download on this PC fills the line's queue past the router,
            // and looks just like the provider. Both facts are said; which
            // one it was is not known from here.
            (Blame::Beyond, Some(m)) => Some((i18n::ov_beyond_busy(m), Tone::Warn)),
            (Blame::Beyond, None) => Some((i18n::ov_beyond().to_string(), Tone::Warn)),
            (Blame::Clean | Blame::Unknown, _) => None,
        }
    };
    View { router, internet, status: verdict }
}

/// The overlay's distances, all from the size of its numbers, `px`: the
/// small and large settings scale the whole card, not only the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The numbers' font height.
    pub value_px: i32,
    /// "ROUTER", "INTERNET": small and quiet beside the numbers.
    pub label_px: i32,
    /// The verdict row.
    pub status_px: i32,
    /// The coloured bar down the left edge.
    pub bar: i32,
    pub pad_x: i32,
    pub pad_y: i32,
    /// Between a label and its number.
    pub label_gap: i32,
    /// Between the router's number and the internet's label.
    pub pair_gap: i32,
    pub row_h: i32,
    pub radius: i32,
}

pub fn layout(px: i32) -> Layout {
    Layout {
        value_px: px,
        label_px: px * 7 / 10,
        status_px: px * 9 / 10,
        bar: (px / 5).max(3),
        pad_x: px * 4 / 5,
        pad_y: px * 2 / 5,
        label_gap: px * 2 / 5,
        pair_gap: px,
        row_h: px * 4 / 3,
        radius: px * 2 / 3,
    }
}

impl Layout {
    /// The window's size around `content_w` pixels of text in `rows` rows.
    pub fn size(&self, content_w: i32, rows: usize) -> (i32, i32) {
        (self.bar + 2 * self.pad_x + content_w, 2 * self.pad_y + self.row_h * rows.max(1) as i32)
    }

    /// Where row `row`'s text sits, as a baseline, for a font of `font_px`:
    /// capitals centred in the row, so rows of different sizes line up.
    pub fn baseline(&self, row: usize, font_px: i32) -> i32 {
        self.pad_y + self.row_h * row as i32 + self.row_h / 2 + font_px * 7 / 20
    }
}

/// The window's alpha for an opacity in percent.
pub fn alpha(opacity_pct: u8) -> u8 {
    let pct = opacity_pct.clamp(crate::settings::OVERLAY_OPACITY_MIN, 100) as u32;
    (pct * 255 / 100) as u8
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
        SIZE, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, EndPaint,
        FillRect, FrameRgn, GetDC, GetMonitorInfoW, GetTextExtentPoint32W, InvalidateRect,
        MonitorFromWindow, ReleaseDC, SelectObject, SetBkMode, SetTextAlign, SetTextColor,
        SetWindowRgn, TextOutW, HDC, HFONT, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, PAINTSTRUCT,
        TA_BASELINE, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
        GetMessageW, PostMessageW, PostQuitMessage, RegisterClassW, SetLayeredWindowAttributes,
        SetTimer, SetWindowPos, ShowWindow, TranslateMessage, HWND_TOPMOST, LWA_ALPHA, MSG,
        SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, WM_CLOSE, WM_DESTROY, WM_PAINT, WM_TIMER,
        WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        WS_EX_TRANSPARENT, WS_POPUP,
    };

    use super::{alpha, layout, origin, reading, view, wanted, Layout, Tone, View};
    use crate::game::Reading;
    use crate::i18n;
    use crate::monitor::{Seen, Shared};
    use crate::probe::netstate::{self, LinkCounters};

    const CLASS: &str = "NetDoctorOverlay";
    const TIMER_ID: usize = 1;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
        COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
    }

    /// The window's palette, as the rest of the app.
    fn tone_rgb(t: Tone) -> (u8, u8, u8) {
        match t {
            Tone::Good => (0x3d, 0xdc, 0x84),
            Tone::Warn => (0xff, 0xc4, 0x4d),
            Tone::Bad => (0xff, 0x5f, 0x5f),
            Tone::Dim => (0x9a, 0xa3, 0xb4),
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
        fonts: Fonts,
        shown: bool,
        /// Sweeps of the last [`crate::game::BLAME_SPAN_S`], with their time.
        history: VecDeque<(f64, Reading)>,
        last_ts: f64,
        counters: Option<(LinkCounters, Instant)>,
        busy: Option<f64>,
        view: View,
        alpha: u8,
        /// The window's size. It only grows while the verdict stays the same,
        /// so "8 ms" turning into "12 ms" does not shake a right-hand corner;
        /// a new verdict fits the card to it again.
        size: (i32, i32),
        /// Where the window was last put, and at what size.
        placed: Option<((i32, i32), (i32, i32))>,
    }

    /// The card's three faces, made for one [`Layout`].
    struct Fonts {
        layout: Layout,
        label: HFONT,
        value: HFONT,
        status: HFONT,
    }

    impl Fonts {
        fn new(px: i32) -> Fonts {
            let layout = layout(px);
            Fonts {
                layout,
                label: make_font(layout.label_px, 600),
                value: make_font(layout.value_px, 700),
                status: make_font(layout.status_px, 600),
            }
        }

        fn delete(&self) {
            unsafe {
                let _ = DeleteObject(self.label);
                let _ = DeleteObject(self.value);
                let _ = DeleteObject(self.status);
            }
        }
    }

    fn make_font(px: i32, weight: i32) -> HFONT {
        let face = wide("Segoe UI");
        // DEFAULT_CHARSET (1) and CLEARTYPE_QUALITY (5).
        unsafe { CreateFontW(-px, 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, PCWSTR(face.as_ptr())) }
    }

    const BG: (u8, u8, u8) = (0x12, 0x14, 0x19);
    const BORDER: (u8, u8, u8) = (0x2c, 0x31, 0x3b);
    const LABEL: (u8, u8, u8) = (0x80, 0x89, 0x9c);
    const VALUE: (u8, u8, u8) = (0xe8, 0xeb, 0xf1);

    fn rgb_of((r, g, b): (u8, u8, u8)) -> COLORREF {
        rgb(r, g, b)
    }

    unsafe fn fill(hdc: HDC, rect: RECT, c: (u8, u8, u8)) {
        unsafe {
            let brush = CreateSolidBrush(rgb_of(c));
            FillRect(hdc, &rect, brush);
            let _ = DeleteObject(brush);
        }
    }

    /// Writes `s` at `at` (left, baseline) when given, and returns its width
    /// either way: measuring and drawing walk the same path, so the card is
    /// always as wide as what is drawn in it.
    unsafe fn run(hdc: HDC, font: HFONT, s: &str, at: Option<(i32, i32, COLORREF)>) -> i32 {
        let units: Vec<u16> = s.encode_utf16().collect();
        let old = unsafe { SelectObject(hdc, font) };
        let mut size = SIZE::default();
        let _ = unsafe { GetTextExtentPoint32W(hdc, &units, &mut size) };
        if let Some((x, y, colour)) = at {
            unsafe {
                SetTextColor(hdc, colour);
                let _ = TextOutW(hdc, x, y, &units);
            }
        }
        unsafe { SelectObject(hdc, old) };
        size.cx
    }

    /// Lays `view` out from `x0`, drawing it when `draw`, and returns the
    /// widest row's width.
    unsafe fn lay_out(hdc: HDC, f: &Fonts, view: &View, x0: i32, draw: bool) -> i32 {
        let l = &f.layout;
        let mut row = 0;
        let mut widest = 0;
        if view.router.is_some() || view.internet.is_some() {
            let y = l.baseline(row, l.value_px);
            let mut x = x0;
            let at = |x: i32, c: (u8, u8, u8)| draw.then(|| (x, y, rgb_of(c)));
            let pairs = [
                view.router.as_ref().map(|v| (i18n::ov_router_label(), v.as_str(), VALUE)),
                view.internet
                    .as_ref()
                    .map(|(v, t)| (i18n::ov_internet_label(), v.as_str(), tone_rgb(*t))),
            ];
            for (i, (label, value, c)) in pairs.into_iter().flatten().enumerate() {
                if i > 0 {
                    x += l.pair_gap;
                }
                x += unsafe { run(hdc, f.label, label, at(x, LABEL)) } + l.label_gap;
                x += unsafe { run(hdc, f.value, value, at(x, c)) };
            }
            widest = widest.max(x - x0);
            row += 1;
        }
        if let Some((s, t)) = &view.status {
            let y = l.baseline(row, l.status_px);
            let at = draw.then(|| (x0, y, rgb_of(tone_rgb(*t))));
            widest = widest.max(unsafe { run(hdc, f.status, s, at) });
        }
        widest
    }

    /// The bounds of the screen the game is on, as `(left, top, right,
    /// bottom)`: the monitor of the window in the foreground, which during a
    /// match is the game. With no foreground window, the primary screen.
    // ponytail: follows the foreground window, so alt-tabbing to a chat on
    // another monitor moves the overlay there too; finding the game's own
    // window by its process would pin it.
    fn game_screen() -> Option<(i32, i32, i32, i32)> {
        unsafe {
            let mon = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(mon, &mut info).as_bool() {
                return None;
            }
            let r = info.rcMonitor;
            Some((r.left, r.top, r.right, r.bottom))
        }
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
            let want = wanted(&settings, self.shared.gaming.load(Ordering::Relaxed));
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
                    self.history.push_back((last.ts, r));
                    let from = last.ts - crate::game::BLAME_SPAN_S;
                    while self.history.front().is_some_and(|(ts, _)| *ts < from) {
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

            let window: Vec<Reading> = self.history.iter().map(|(_, r)| *r).collect();
            let next = view(seen, now_reading, crate::game::blame(&window), self.busy, &settings);

            let px = settings.overlay_size.font_px();
            let resized = px != self.fonts.layout.value_px;
            if resized {
                self.fonts.delete();
                self.fonts = Fonts::new(px);
            }
            let a = alpha(settings.overlay_opacity);
            if a != self.alpha {
                unsafe {
                    let _ = SetLayeredWindowAttributes(self.hwnd, COLORREF(0), a, LWA_ALPHA);
                }
                self.alpha = a;
            }
            if next != self.view || !self.shown || resized {
                let fit = self.measure(&next);
                let same_shape = !resized
                    && next.status == self.view.status
                    && next.rows() == self.view.rows()
                    && next.router.is_some() == self.view.router.is_some();
                self.size = if same_shape { (fit.0.max(self.size.0), fit.1) } else { fit };
                self.view = next;
                unsafe {
                    let _ = InvalidateRect(self.hwnd, None, true);
                }
            }
            if let Some(screen) = game_screen() {
                let at = origin(screen, settings.game_overlay_corner, self.size);
                if self.placed != Some((at, self.size)) {
                    let (w, h) = self.size;
                    let r = self.fonts.layout.radius;
                    unsafe {
                        let _ =
                            SetWindowPos(self.hwnd, HWND_TOPMOST, at.0, at.1, w, h, SWP_NOACTIVATE);
                        // The system owns the region once it is set.
                        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, r, r);
                        SetWindowRgn(self.hwnd, rgn, true);
                    }
                    self.placed = Some((at, self.size));
                }
            }
            if !self.shown {
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                }
                self.shown = true;
            }
        }

        /// The window size `v` needs in the current fonts.
        fn measure(&self, v: &View) -> (i32, i32) {
            let content = unsafe {
                let hdc = GetDC(self.hwnd);
                let w = lay_out(hdc, &self.fonts, v, 0, false);
                ReleaseDC(self.hwnd, hdc);
                w
            };
            self.fonts.layout.size(content, v.rows())
        }

        fn paint(&self) {
            let l = self.fonts.layout;
            let (w, h) = self.size;
            unsafe {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(self.hwnd, &mut ps);
                fill(hdc, RECT { left: 0, top: 0, right: w, bottom: h }, BG);
                // The state at a glance, down the left edge.
                fill(
                    hdc,
                    RECT { left: 0, top: 0, right: l.bar, bottom: h },
                    tone_rgb(self.view.accent()),
                );
                let edge = CreateRoundRectRgn(0, 0, w, h, l.radius, l.radius);
                let border = CreateSolidBrush(rgb_of(BORDER));
                let _ = FrameRgn(hdc, edge, border, 1, 1);
                let _ = DeleteObject(border);
                let _ = DeleteObject(edge);

                SetBkMode(hdc, TRANSPARENT);
                SetTextAlign(hdc, TA_BASELINE);
                lay_out(hdc, &self.fonts, &self.view, l.bar + l.pad_x, true);
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
                0,
                0,
                1,
                1,
                None,
                None,
                hinstance,
                None,
            )
        }
        .ok()?;
        // The window's size, font and alpha are the settings', set on the
        // first tick before the window is shown.
        let state = State {
            hwnd,
            shared,
            fonts: Fonts::new(crate::settings::OverlaySize::Medium.font_px()),
            alpha: 0,
            size: (1, 1),
            placed: None,
            shown: false,
            history: VecDeque::new(),
            last_ts: 0.0,
            counters: None,
            busy: None,
            view: View { router: None, internet: None, status: None },
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
                state.fonts.delete();
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

    /// The full overlay's internet reading and verdict.
    fn two(
        seen: Seen<'_>,
        now: Option<Reading>,
        blame: Blame,
        busy: Option<f64>,
        s: &Settings,
    ) -> [(String, Tone); 2] {
        let v = view(seen, now, blame, busy, s);
        [v.internet.expect("internet shown"), v.status.expect("verdict shown")]
    }

    #[test]
    fn nothing_measured_says_so_instead_of_a_ping() {
        let s = Settings::default();
        let v = view(Seen::Paused, at(3.0, 20.0), Blame::Clean, None, &s);
        let status = (i18n::ov_not_measuring().to_string(), Tone::Dim);
        assert_eq!(v, View { router: None, internet: None, status: Some(status) });
        assert_eq!((v.rows(), v.accent()), (1, Tone::Dim));
    }

    #[test]
    fn the_compact_overlay_keeps_the_ping_and_never_hides_an_outage() {
        let mut s = Settings::default();
        let ok = Seen::Verdict(Status::Ok, "");
        let down = Seen::Verdict(Status::IspDown, "");
        let good = Some(("22 ms".to_string(), Tone::Good));

        s.overlay_content = OverlayContent::Ping;
        let v = view(ok, at(3.0, 22.0), Blame::Local, None, &s);
        assert_eq!(v, View { router: Some("3 ms".into()), internet: good.clone(), status: None });

        s.overlay_content = OverlayContent::Internet;
        let v = view(ok, at(3.0, 22.0), Blame::Local, None, &s);
        assert_eq!(v, View { router: None, internet: good, status: None }, "no router either");
        assert_eq!(v.rows(), 1);

        // Down is said instead of "Internet —".
        let v = view(down, None, Blame::Beyond, None, &s);
        let out = (Status::IspDown.headline().to_string(), Tone::Bad);
        assert_eq!(v, View { router: None, internet: None, status: Some(out) });
        assert_eq!(v.accent(), Tone::Bad);
    }

    #[test]
    fn the_bar_takes_the_verdicts_colour_over_the_pings() {
        let s = Settings::default();
        let ok = Seen::Verdict(Status::Ok, "");
        // A good ping with a spike on this side: the bar says the spike.
        assert_eq!(view(ok, at(3.0, 22.0), Blame::Local, None, &s).accent(), Tone::Bad);
        assert_eq!(view(ok, at(3.0, 22.0), Blame::Clean, None, &s).accent(), Tone::Good);
    }

    #[test]
    fn the_card_scales_with_its_size_setting() {
        let medium = layout(15);
        // Two rows are taller than one, and a bigger font a bigger card
        // around the same text.
        assert!(medium.size(200, 2).1 > medium.size(200, 1).1);
        assert!(layout(20).size(200, 2) > medium.size(200, 2));
        // The text starts past the bar and the padding, and nothing is
        // narrower than what it holds.
        assert_eq!(medium.size(200, 1).0, 200 + medium.bar + 2 * medium.pad_x);
        // A second row's baseline is below the first's.
        assert!(medium.baseline(1, medium.status_px) > medium.baseline(0, medium.value_px));
        // The labels are smaller than the numbers they name.
        assert!(medium.label_px < medium.value_px);
    }

    #[test]
    fn opacity_cannot_go_below_readable() {
        assert_eq!(alpha(100), 255);
        assert_eq!(alpha(0), alpha(crate::settings::OVERLAY_OPACITY_MIN));
        assert_eq!(alpha(250), 255);
    }

    #[test]
    fn the_overlay_is_up_in_games_and_nowhere_else() {
        let mut s = Settings::default();
        assert!(wanted(&s, true));
        assert!(!wanted(&s, false), "not over a browser");
        s.game_overlay = false;
        assert!(!wanted(&s, true), "off is off, games or not");
    }

    #[test]
    fn the_overlay_sits_in_the_chosen_corner_of_the_games_screen() {
        // A second monitor right of a 2560x1080 one.
        let screen = (2560, 0, 4480, 1080);
        let size = (360, 48);
        assert_eq!(origin(screen, Corner::TopLeft, size), (2572, 12));
        assert_eq!(origin(screen, Corner::TopRight, size), (4108, 12));
        assert_eq!(origin(screen, Corner::BottomLeft, size), (2572, 1020));
        assert_eq!(origin(screen, Corner::BottomRight, size), (4108, 1020));
    }

    #[test]
    fn a_spike_on_this_side_outranks_everything_short_of_an_outage() {
        let s = Settings::default();
        let ok = Seen::Verdict(Status::Ok, "");
        let [first, second] = two(ok, at(3.0, 22.0), Blame::Local, Some(40.0), &s);
        assert_eq!(first.1, Tone::Good);
        assert_eq!(second.1, Tone::Bad);
        // Calm says nothing, busy or not: the green bar says it, and
        // traffic that costs no ping is no problem.
        for busy in [Some(12.0), Some(0.3), None] {
            let v = view(ok, at(3.0, 22.0), Blame::Clean, busy, &s);
            assert_eq!((v.rows(), v.status), (1, None));
        }
        // Too few readings to judge: nothing to say yet either.
        assert_eq!(view(ok, at(3.0, 22.0), Blame::Unknown, None, &s).status, None);
        // A spike past the router while this PC is busy names the traffic:
        // a download fills the line's queue and looks like the provider.
        let [_, queued] = two(ok, at(3.0, 140.0), Blame::Beyond, Some(40.0), &s);
        assert_eq!((queued.0, queued.1), (i18n::ov_beyond_busy(40.0), Tone::Warn));
        let [_, far] = two(ok, at(3.0, 140.0), Blame::Beyond, Some(0.3), &s);
        assert_eq!(far.0, i18n::ov_beyond());
        // An outage is said as one.
        let down = Seen::Verdict(Status::IspDown, "");
        let [_, out] = two(down, None, Blame::Beyond, None, &s);
        assert_eq!((out.0.as_str(), out.1), (Status::IspDown.headline(), Tone::Bad));
    }
}
