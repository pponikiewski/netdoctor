//! The notification-area icon and the Windows notifications that go with it.
//!
//! The app mostly runs hidden: autostart starts it with `--minimised`, and
//! everything it had to say lived inside a window nobody could see. Outage
//! toasts were drawn in egui, and the only way back to the window was to
//! launch the exe again. This is the part that stays visible while the window
//! is not.
//!
//! It runs on its own thread with its own hidden window and message loop,
//! apart from winit, and reads the monitor's [`Shared`] directly. An egui app
//! whose window is hidden cannot be relied on to get frames, and a hidden
//! window is exactly the situation this exists for.

use crate::i18n;
#[cfg(test)]
use crate::monitor::Snapshot;
use crate::monitor::{Notice, Seen, Status};
#[cfg(test)]
use std::time::Duration;

/// What the icon shows.
///
/// `Unknown` is grey and means "not measuring": no sweep yet, or sampling
/// paused by the user or by a job. A green icon over a paused monitor would
/// claim a healthy line that nobody is measuring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Unknown,
    Ok,
    Slow,
    Down,
}

impl Level {
    fn of(seen: Seen) -> Level {
        match seen {
            Seen::Paused | Seen::Waiting | Seen::Blind | Seen::Stale(_) => Level::Unknown,
            Seen::Verdict(Status::Ok, _) => Level::Ok,
            Seen::Verdict(Status::Degraded, _) => Level::Slow,
            Seen::Verdict(..) => Level::Down,
        }
    }

    const ALL: [Level; 4] = [Level::Unknown, Level::Ok, Level::Slow, Level::Down];

    fn index(self) -> usize {
        self as usize
    }

    /// Fill colour, as RGB. The same hues as the window's palette.
    fn rgb(self) -> (u8, u8, u8) {
        match self {
            Level::Unknown => (0x9a, 0xa3, 0xb4),
            Level::Ok => (0x3d, 0xdc, 0x84),
            Level::Slow => (0xff, 0xc4, 0x4d),
            Level::Down => (0xff, 0x5f, 0x5f),
        }
    }
}

/// The hover text: the verdict and its note, or why there is none.
fn tooltip(seen: Seen) -> String {
    match seen {
        Seen::Paused => i18n::tray_paused().to_string(),
        Seen::Waiting => i18n::tray_waiting().to_string(),
        Seen::Blind => format!("NetDoctor: {}", i18n::mon_blind()),
        Seen::Stale(age) => i18n::tray_stale(age),
        Seen::Verdict(status, "") => format!("NetDoctor: {}", status.headline()),
        Seen::Verdict(status, note) => format!("NetDoctor: {}\n{note}", status.headline()),
    }
}

/// The balloon for a notice: title and body.
fn balloon_text(notice: &Notice) -> (String, String) {
    match notice {
        Notice::Down { status, note } => (status.headline().to_string(), note.clone()),
        Notice::Up { secs } => ("NetDoctor".to_string(), i18n::toast_restored(*secs)),
    }
}

/// A 32-bit icon of `size` pixels square: a filled circle with a darker rim,
/// so the yellow still reads on a light taskbar. Returns the AND mask and the
/// BGRA colour bits in the layout `CreateIcon` takes.
fn circle_bits(size: usize, (r, g, b): (u8, u8, u8)) -> (Vec<u8>, Vec<u8>) {
    let stride = size.div_ceil(16) * 2;
    let mut mask = vec![0xffu8; stride * size];
    let mut colour = vec![0u8; size * size * 4];
    let centre = (size as f64 - 1.0) / 2.0;
    let radius = size as f64 * 0.42;
    let dark = |c: u8| (c as f64 * 0.55) as u8;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f64 - centre).powi(2) + (y as f64 - centre).powi(2)).sqrt();
            if d > radius {
                continue;
            }
            // Opaque here: clear the mask bit.
            mask[y * stride + x / 8] &= !(0x80 >> (x % 8));
            let (pr, pg, pb) =
                if d > radius - 1.3 { (dark(r), dark(g), dark(b)) } else { (r, g, b) };
            let at = (y * size + x) * 4;
            colour[at..at + 4].copy_from_slice(&[pb, pg, pr, 0xff]);
        }
    }
    (mask, colour)
}

/// Copies `s` into a fixed UTF-16 field, cut to fit, always terminated.
fn fill(dst: &mut [u16], s: &str) {
    let room = dst.len().saturating_sub(1);
    let mut n = 0;
    for (slot, unit) in dst.iter_mut().zip(s.encode_utf16().take(room)) {
        *slot = unit;
        n += 1;
    }
    if let Some(end) = dst.get_mut(n) {
        *end = 0;
    }
}

#[cfg(windows)]
pub use imp::{remove_after_crash, Tray};

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Duration;

    use crossbeam_channel::Receiver;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        GetLastError, BOOL, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT,
        TRUE, WPARAM,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING,
        NIM_ADD, NIM_DELETE, NIM_MODIFY, NIN_BALLOONSHOW, NIN_BALLOONUSERCLICK, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
        DestroyMenu, DestroyWindow, DispatchMessageW, EnumWindows, GetCursorPos, GetMessageW,
        GetSystemMetrics, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, PostQuitMessage,
        RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetTimer, ShowWindow,
        TrackPopupMenu, TranslateMessage, HICON, MF_SEPARATOR, MF_STRING, MSG, SM_CXSMICON,
        SW_RESTORE, SW_SHOWMINNOACTIVE, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP,
        WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
        WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
    };

    use super::{balloon_text, circle_bits, fill, tooltip, Level, Seen};
    use crate::monitor::{Notice, Shared};

    /// The window class. A test finds the tray window by it.
    const CLASS: &str = "NetDoctorTray";
    /// What the icon sends its clicks as.
    const WM_TRAY: u32 = WM_APP + 1;
    const ICON_ID: u32 = 1;
    const TIMER_ID: usize = 1;
    /// Menu commands. Also what a test sends as `WM_COMMAND`.
    const CMD_OPEN: usize = 1;
    const CMD_QUIT: usize = 2;

    /// `TaskbarCreated`, which Explorer broadcasts when it restarts and has
    /// forgotten every icon. Registered once, read by the window procedure.
    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

    /// The window the icon is registered under while it is registered, for
    /// [`remove_after_crash`]. `0` when there is no icon.
    static ICON_HWND: AtomicIsize = AtomicIsize::new(0);

    /// Notifications Windows took from us, and ones it reports having shown.
    /// Only the live test reads them: the two differ exactly when Windows
    /// swallowed a notification it had accepted.
    #[cfg(test)]
    pub(super) static BALLOONS_ACCEPTED: AtomicU32 = AtomicU32::new(0);
    #[cfg(test)]
    pub(super) static BALLOONS_SHOWN: AtomicU32 = AtomicU32::new(0);

    fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON_ID,
            ..Default::default()
        }
    }

    /// Removes the icon from the panic hook.
    ///
    /// The release build aborts on panic, so no destructor runs and the icon
    /// stayed in the notification area, looking alive, until the user moved
    /// the pointer over it. `Shell_NotifyIconW` takes the window and the id
    /// from any thread, which is all a dying process has time for.
    pub fn remove_after_crash() {
        let hwnd = ICON_HWND.swap(0, Ordering::Relaxed);
        if hwnd != 0 {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(HWND(hwnd as *mut _)));
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The icon, owned by the app. Dropping it removes the icon and stops the
    /// thread, which is what keeps a closed app from leaving a dead icon
    /// behind until the user happens to hover over it.
    pub struct Tray {
        /// The tray window, as a number: `HWND` is not `Send`, and this only
        /// ever travels back to be posted to.
        hwnd: isize,
        thread: Option<JoinHandle<()>>,
    }

    impl Tray {
        /// `None` when the window or the icon could not be created. The app
        /// then runs as it did before there was an icon: no hiding on close,
        /// because there would be nothing to bring it back with.
        pub fn start(
            shared: Arc<Shared>,
            notices: Receiver<Notice>,
            quit: Arc<AtomicBool>,
        ) -> Option<Tray> {
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Option<isize>>();
            let thread = std::thread::Builder::new()
                .name("netdoctor-tray".into())
                .spawn(move || {
                    let hwnd = unsafe { create(shared, notices, quit) };
                    let _ = ready_tx.send(hwnd.map(|h| h.0 as isize));
                    if hwnd.is_some() {
                        unsafe { pump() };
                    }
                })
                .ok()?;
            match ready_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(Some(hwnd)) => Some(Tray { hwnd, thread: Some(thread) }),
                _ => None,
            }
        }
    }

    impl Tray {
        /// The tray window, for the tests that look for its icon.
        #[cfg(test)]
        pub(super) fn hwnd(&self) -> isize {
            self.hwnd
        }
    }

    impl Drop for Tray {
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
        icons: [HICON; 4],
        shown: Level,
        tip: String,
        shared: Arc<Shared>,
        notices: Receiver<Notice>,
        quit: Arc<AtomicBool>,
    }

    thread_local! {
        // The window procedure runs on this thread only, so the state lives
        // here rather than behind a pointer stashed in the window.
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    /// Runs `f` on the state, unless it is already borrowed. It can be: the
    /// menu's modal loop dispatches timer messages from inside a handler.
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
        fn data(&self) -> NOTIFYICONDATAW {
            icon_data(self.hwnd)
        }

        /// Reads the monitor and works out what the icon should say.
        fn read(&self) -> (Level, String) {
            // Cloned so the monitor's lock is not held while the text is built.
            let last = self.shared.last.lock().unwrap_or_else(|p| p.into_inner()).clone();
            let interval =
                self.shared.settings.lock().unwrap_or_else(|p| p.into_inner()).interval();
            let seen = Seen::of(&last, !self.shared.paused(), crate::store::now(), interval);
            (Level::of(seen), tooltip(seen))
        }

        fn add(&self) -> bool {
            let mut d = self.data();
            d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            d.uCallbackMessage = WM_TRAY;
            d.hIcon = self.icons[self.shown.index()];
            fill(&mut d.szTip, &self.tip);
            let added = unsafe { Shell_NotifyIconW(NIM_ADD, &d) }.as_bool();
            if added {
                ICON_HWND.store(self.hwnd.0 as isize, Ordering::Relaxed);
            }
            added
        }

        fn tick(&mut self) {
            let (level, tip) = self.read();
            if level != self.shown || tip != self.tip {
                self.shown = level;
                self.tip = tip;
                let mut d = self.data();
                d.uFlags = NIF_ICON | NIF_TIP;
                d.hIcon = self.icons[level.index()];
                fill(&mut d.szTip, &self.tip);
                unsafe {
                    let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
                }
            }
            while let Ok(notice) = self.notices.try_recv() {
                self.balloon(&notice);
            }
        }

        fn balloon(&self, notice: &Notice) {
            let (title, body) = balloon_text(notice);
            let mut d = self.data();
            d.uFlags = NIF_INFO;
            d.dwInfoFlags = match notice {
                Notice::Down { .. } => NIIF_WARNING,
                Notice::Up { .. } => NIIF_INFO,
            };
            fill(&mut d.szInfoTitle, &title);
            // An empty body makes Windows drop the balloon altogether.
            fill(&mut d.szInfo, if body.is_empty() { &title } else { &body });
            let accepted = unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) }.as_bool();
            #[cfg(test)]
            if accepted {
                BALLOONS_ACCEPTED.fetch_add(1, Ordering::Relaxed);
            }
            let _ = accepted;
        }

        fn remove(&self) {
            ICON_HWND.store(0, Ordering::Relaxed);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
            }
        }
    }

    unsafe fn make_icon(hinstance: HINSTANCE, level: Level) -> Option<HICON> {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64);
        let (mask, colour) = circle_bits(size as usize, level.rgb());
        unsafe { CreateIcon(hinstance, size, size, 1, 32, mask.as_ptr(), colour.as_ptr()) }.ok()
    }

    /// Creates the window and the icon, and parks the state on this thread.
    ///
    /// # Safety
    /// Call once, on the thread that will then run [`pump`].
    unsafe fn create(
        shared: Arc<Shared>,
        notices: Receiver<Notice>,
        quit: Arc<AtomicBool>,
    ) -> Option<HWND> {
        let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }.ok()?.into();
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        // Registered once per process; a second tray (a test, or an app
        // started twice in one process) finds the class already there.
        if unsafe { RegisterClassW(&wc) } == 0
            && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
        {
            return None;
        }
        let taskbar = wide("TaskbarCreated");
        TASKBAR_CREATED
            .store(unsafe { RegisterWindowMessageW(PCWSTR(taskbar.as_ptr())) }, Ordering::Relaxed);

        // A top-level window that is never shown, not a message-only one:
        // `TaskbarCreated` is broadcast to top-level windows only. No title,
        // which is also what keeps `main_window` from mistaking it for the
        // app's own.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                hinstance,
                None,
            )
        }
        .ok()?;

        let mut icons = [HICON::default(); 4];
        for level in Level::ALL {
            icons[level.index()] = unsafe { make_icon(hinstance, level) }?;
        }
        let mut state =
            State { hwnd, icons, shown: Level::Unknown, tip: String::new(), shared, notices, quit };
        let (level, tip) = state.read();
        state.shown = level;
        state.tip = tip;
        if !state.add() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return None;
        }
        STATE.with(|cell| *cell.borrow_mut() = Some(state));
        unsafe { SetTimer(hwnd, TIMER_ID, 1000, None) };
        Some(hwnd)
    }

    /// The message loop. Returns when the window is destroyed.
    ///
    /// # Safety
    /// Only on the thread that ran [`create`].
    unsafe fn pump() {
        let mut msg = MSG::default();
        // `GetMessageW` returns -1 on error, which is "true" as a BOOL; only
        // a positive value is a message.
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().take() {
                for icon in state.icons {
                    unsafe {
                        let _ = DestroyIcon(icon);
                    }
                }
            }
        });
    }

    /// The app's own window: this process's, and titled as the app's.
    fn main_window() -> Option<HWND> {
        struct Search {
            pid: u32,
            found: HWND,
        }
        unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
            // SAFETY: `param` is the `Search` below, alive for the whole
            // enumeration.
            let search = unsafe { &mut *(param.0 as *mut Search) };
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if pid == search.pid && crate::single::is_app_window(hwnd) {
                search.found = hwnd;
                return BOOL(0);
            }
            TRUE
        }
        let mut search = Search { pid: unsafe { GetCurrentProcessId() }, found: HWND::default() };
        unsafe {
            let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
        }
        (!search.found.0.is_null()).then_some(search.found)
    }

    fn open_window() {
        if let Some(hwnd) = main_window() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }

    /// Lets the app close for real, then asks its window to.
    ///
    /// A hidden window is shown minimised first. eframe acts on a close only
    /// inside a frame, and Windows does not paint a hidden window, so a
    /// WM_CLOSE sent to one waited forever. Minimised and not activated, it
    /// gets its frame without appearing anywhere but, for a moment, on the
    /// taskbar.
    fn quit() {
        with_state(|s| s.quit.store(true, Ordering::Relaxed));
        if let Some(hwnd) = main_window() {
            unsafe {
                if !IsWindowVisible(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
                }
                let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn show_menu(hwnd: HWND) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let open = wide(crate::i18n::tray_open());
            let quit = wide(crate::i18n::tray_quit());
            let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN, PCWSTR(open.as_ptr()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, PCWSTR(quit.as_ptr()));
            let mut at = POINT::default();
            let _ = GetCursorPos(&mut at);
            // Both documented quirks of a tray menu: without the first it
            // does not close when the user clicks elsewhere, without the
            // second it fails to open on every other click.
            let _ = SetForegroundWindow(hwnd);
            let _ =
                TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, at.x, at.y, 0, hwnd, None);
            let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_TIMER => with_state(State::tick),
            WM_TRAY => match (lparam.0 as u32) & 0xffff {
                WM_LBUTTONUP | NIN_BALLOONUSERCLICK => open_window(),
                NIN_BALLOONSHOW => {
                    #[cfg(test)]
                    BALLOONS_SHOWN.fetch_add(1, Ordering::Relaxed);
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            },
            WM_COMMAND => match wparam.0 & 0xffff {
                CMD_OPEN => open_window(),
                CMD_QUIT => quit(),
                _ => {}
            },
            WM_CLOSE => {
                with_state(|s| s.remove());
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            WM_DESTROY => unsafe { PostQuitMessage(0) },
            m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                with_state(|s| {
                    s.add();
                });
            }
            _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
        LRESULT(0)
    }
}

#[cfg(all(test, windows))]
mod live_tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIM_MODIFY, NOTIFYICONDATAW};

    use super::imp::{remove_after_crash, Tray};
    use crate::monitor::Shared;
    use crate::settings::Settings;

    /// Whether Windows still has an icon under this window: a modify with no
    /// flags succeeds exactly when it does.
    fn icon_exists(hwnd: isize) -> bool {
        let d = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: HWND(hwnd as *mut _),
            uID: 1,
            ..Default::default()
        };
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) }.as_bool()
    }

    fn start() -> Tray {
        let shared = Arc::new(Shared::new(Settings::default()));
        let (_tx, rx) = crossbeam_channel::bounded(1);
        Tray::start(shared, rx, Arc::new(AtomicBool::new(false)))
            .expect("a notification area to put the icon in")
    }

    #[test]
    #[ignore = "shows a real Windows notification; needs a desktop session"]
    fn a_notice_reaches_windows_as_a_notification() {
        use std::sync::atomic::Ordering;
        use std::time::{Duration, Instant};

        use super::imp::{BALLOONS_ACCEPTED, BALLOONS_SHOWN};
        use crate::monitor::{Notice, Status};

        let shared = Arc::new(Shared::new(Settings::default()));
        let (tx, rx) = crossbeam_channel::bounded(4);
        let tray =
            Tray::start(shared, rx, Arc::new(AtomicBool::new(false))).expect("a notification area");
        tx.send(Notice::Down { status: Status::IspDown, note: "netdoctor test".into() })
            .expect("the tray is listening");

        let wait_for = |counter: &std::sync::atomic::AtomicU32, secs: u64| {
            let until = Instant::now() + Duration::from_secs(secs);
            while counter.load(Ordering::Relaxed) == 0 && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
            counter.load(Ordering::Relaxed)
        };
        let accepted = wait_for(&BALLOONS_ACCEPTED, 5);
        let shown = wait_for(&BALLOONS_SHOWN, 8);
        println!("accepted by Windows: {accepted}, shown by Windows: {shown}");
        drop(tray);
        assert_eq!(accepted, 1, "the tray handed the notice to Windows");
    }

    /// Reconnects the Wi-Fi when dropped, so a failing assertion cannot
    /// leave the machine offline.
    struct Reconnect {
        iface: String,
        profile: String,
    }

    impl Reconnect {
        fn online() -> bool {
            std::process::Command::new("ping")
                .args(["-n", "1", "-w", "1000", "1.1.1.1"])
                .output()
                .is_ok_and(|o| o.status.success())
        }

        fn now(&self) -> bool {
            for _ in 0..15 {
                if Self::online() {
                    return true;
                }
                let _ = std::process::Command::new("netsh")
                    .args(["wlan", "connect"])
                    .arg(format!("name={}", self.profile))
                    .arg(format!("interface={}", self.iface))
                    .output();
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            Self::online()
        }
    }

    impl Drop for Reconnect {
        fn drop(&mut self) {
            self.now();
        }
    }

    /// The whole chain on a real outage: the monitor sees the line go, the
    /// history records it, the tray says so through Windows, and says it is
    /// back promptly once it is, with a length that matches what happened.
    ///
    /// Cuts the Wi-Fi for about ten seconds. Needs the interface and the
    /// profile to reconnect to, in `NETDOCTOR_TEST_WIFI_IFACE` and
    /// `NETDOCTOR_TEST_WIFI_PROFILE`. Writes to a scratch database, not the
    /// user's history.
    #[test]
    #[ignore = "disconnects the Wi-Fi for about ten seconds"]
    fn a_real_outage_is_announced_and_so_is_its_end() {
        use std::sync::atomic::Ordering;
        use std::time::{Duration, Instant};

        use super::imp::BALLOONS_ACCEPTED;
        use crate::monitor::Monitor;
        use crate::store::Store;

        let iface = std::env::var("NETDOCTOR_TEST_WIFI_IFACE").expect("NETDOCTOR_TEST_WIFI_IFACE");
        let profile =
            std::env::var("NETDOCTOR_TEST_WIFI_PROFILE").expect("NETDOCTOR_TEST_WIFI_PROFILE");
        let db = std::env::temp_dir().join(format!("netdoctor-outage-{}.db", std::process::id()));
        let store = Arc::new(Store::open(&db).expect("a scratch database"));
        let monitor = Monitor::start(Arc::clone(&store), Settings::default());
        let tray = Tray::start(
            Arc::clone(&monitor.shared),
            monitor.notices.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("a notification area");
        BALLOONS_ACCEPTED.store(0, Ordering::Relaxed);

        let wait_for = |n: u32, secs: u64| {
            let until = Instant::now() + Duration::from_secs(secs);
            while BALLOONS_ACCEPTED.load(Ordering::Relaxed) < n && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(100));
            }
            BALLOONS_ACCEPTED.load(Ordering::Relaxed) >= n
        };

        std::thread::sleep(Duration::from_secs(8));
        assert!(Reconnect::online(), "the test starts on a working line");
        assert_eq!(BALLOONS_ACCEPTED.load(Ordering::Relaxed), 0, "no outage before the cut");

        let guard = Reconnect { iface: iface.clone(), profile };
        let cut = Instant::now();
        let _ = std::process::Command::new("netsh")
            .args(["wlan", "disconnect"])
            .arg(format!("interface={iface}"))
            .output();
        let down = wait_for(1, 20);
        let down_after = cut.elapsed();
        assert!(guard.now(), "the line came back");
        let back = Instant::now();
        let up = wait_for(2, 40);
        let up_after = back.elapsed();
        drop(guard);
        drop(tray);
        drop(monitor);

        let events = store.recent_events(10);
        // Closed before it is deleted: Windows will not remove an open file,
        // and WAL leaves two more beside it.
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let mut path = db.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(path);
        }
        println!(
            "down notified: {down} after {down_after:?}; up notified: {up} {up_after:?} after the line was back; line was cut for {:?}; recorded: {:?}",
            back - cut,
            events.iter().map(|e| (e.kind.clone(), e.duration_s())).collect::<Vec<_>>()
        );
        assert!(down, "the outage was announced");
        assert!(up, "its end was announced");
        assert!(up_after < Duration::from_secs(15), "restored promptly, not a minute late");
        assert_eq!(events.len(), 1, "one outage recorded");
        let recorded = events[0].duration_s().expect("the outage was closed");
        let cut_for = (back - cut).as_secs_f64();
        assert!(
            recorded < cut_for + 15.0,
            "recorded {recorded:.0} s for a {cut_for:.0} s cut: the outage ran on after the line was back"
        );
    }

    #[test]
    #[ignore = "puts a real icon in the notification area; needs a desktop session"]
    fn the_icon_goes_away_on_a_crash_on_close_and_can_come_back() {
        // A crash: no destructor runs, only the panic hook.
        let tray = start();
        let hwnd = tray.hwnd();
        assert!(icon_exists(hwnd), "the icon is registered");
        remove_after_crash();
        assert!(!icon_exists(hwnd), "the panic hook removed it");
        drop(tray);

        // A second tray in the same process: the window class is already
        // registered, which used to make `start` give up.
        let tray = start();
        let hwnd = tray.hwnd();
        assert!(icon_exists(hwnd));
        drop(tray);
        assert!(!icon_exists(hwnd), "closing the app removes the icon");
    }
}

/// Without Windows there is no notification area to put an icon in.
#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
pub fn remove_after_crash() {}

#[cfg(not(windows))]
impl Tray {
    pub fn start(
        _shared: std::sync::Arc<crate::monitor::Shared>,
        _notices: crossbeam_channel::Receiver<Notice>,
        _quit: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Option<Tray> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_000_000.0;
    const SECOND: Duration = Duration::from_secs(1);

    /// A sweep taken `age` seconds before `NOW`.
    fn swept(status: Status, note: &str, age: f64) -> Snapshot {
        Snapshot { ts: NOW - age, status, note: note.into(), ..Snapshot::default() }
    }

    #[test]
    fn the_icon_colour_follows_the_verdict() {
        let level = |status| Level::of(Seen::of(&swept(status, "", 0.5), true, NOW, SECOND));
        assert_eq!(level(Status::Ok), Level::Ok);
        assert_eq!(level(Status::Degraded), Level::Slow);
        for down in [Status::DnsFail, Status::IspDown, Status::LanDown, Status::AdapterDown] {
            assert_eq!(level(down), Level::Down, "{down:?}");
        }
    }

    #[test]
    fn a_paused_or_unstarted_monitor_is_grey_not_green() {
        // The snapshot still says "Ok" from before the pause. Showing it
        // would claim a line nobody is measuring is healthy.
        let before_pause = swept(Status::Ok, "", 0.5);
        let paused = Seen::of(&before_pause, false, NOW, SECOND);
        let none_yet = Snapshot::default();
        let waiting = Seen::of(&none_yet, true, NOW, SECOND);
        assert_eq!(Level::of(paused), Level::Unknown);
        assert_eq!(Level::of(waiting), Level::Unknown);
        assert_eq!(tooltip(paused), i18n::tray_paused());
        assert_eq!(tooltip(waiting), i18n::tray_waiting());
    }

    #[test]
    fn a_sweep_that_sent_nothing_is_grey_and_says_so() {
        // A blind snapshot carries the default status, which is "Ok".
        let snap = Snapshot { blind: Some("no handle".into()), ..swept(Status::Ok, "", 0.5) };
        let blind = Seen::of(&snap, true, NOW, SECOND);
        assert_eq!(Level::of(blind), Level::Unknown);
        assert!(tooltip(blind).contains(i18n::mon_blind()));
        assert!(!tooltip(blind).contains(Status::Ok.headline()));
    }

    #[test]
    fn a_reading_nobody_has_refreshed_is_grey_not_the_colour_it_had() {
        // The sweep thread stalled two minutes ago with the line healthy.
        // The icon stayed green for as long as the process lived.
        let stalled = swept(Status::Ok, "", 120.0);
        let seen = Seen::of(&stalled, true, NOW, SECOND);
        assert_eq!(Level::of(seen), Level::Unknown);
        assert!(!tooltip(seen).contains(Status::Ok.headline()), "{}", tooltip(seen));
        assert!(tooltip(seen).contains("120"), "the tooltip says how old: {}", tooltip(seen));
        // In the current language only: switching it here would race the
        // tests running beside this one.
        let tip = tooltip(Seen::Stale(86_400.0 * 30.0));
        assert!(tip.encode_utf16().count() < 128, "szTip holds 127 units: {tip}");

        // A slow sweep is not a stalled one: a few seconds late at a one
        // second interval is still the latest word.
        let late = swept(Status::Ok, "", 4.0);
        assert_eq!(Level::of(Seen::of(&late, true, NOW, SECOND)), Level::Ok);
        // And "a few intervals" scales with the interval.
        let slow = swept(Status::Ok, "", 40.0);
        assert_eq!(Level::of(Seen::of(&slow, true, NOW, Duration::from_secs(10))), Level::Ok);
    }

    #[test]
    fn the_tooltip_carries_the_verdict_and_its_note() {
        let snap = swept(Status::IspDown, "router answers", 0.5);
        let tip = tooltip(Seen::of(&snap, true, NOW, SECOND));
        assert!(tip.contains(Status::IspDown.headline()));
        assert!(tip.contains("router answers"));
    }

    #[test]
    fn a_long_text_is_cut_to_fit_and_always_terminated() {
        let mut field = [0xffffu16; 8];
        fill(&mut field, "a string far longer than eight units");
        assert_eq!(field[7], 0, "the last slot is the terminator");
        assert_eq!(String::from_utf16_lossy(&field[..7]), "a strin");

        let mut field = [0xffffu16; 8];
        fill(&mut field, "ok");
        assert_eq!(&field[..3], &[b'o' as u16, b'k' as u16, 0]);
    }

    #[test]
    fn the_icon_is_a_circle_with_a_transparent_outside() {
        let (mask, colour) = circle_bits(16, (10, 20, 30));
        assert_eq!(mask.len(), 2 * 16, "a 16-pixel row of mask is one WORD");
        assert_eq!(colour.len(), 16 * 16 * 4);
        // A corner is outside the circle: masked out and fully transparent.
        assert_eq!(mask[0] & 0x80, 0x80);
        assert_eq!(&colour[..4], &[0, 0, 0, 0]);
        // The centre is inside, in the colour asked for, stored BGRA.
        let at = (8 * 16 + 8) * 4;
        assert_eq!(&colour[at..at + 4], &[30, 20, 10, 0xff]);
        assert_eq!(mask[8 * 2 + 1] & 0x80, 0, "the centre is opaque");
    }

    #[test]
    fn a_restored_line_says_how_long_it_was_down() {
        let (_, body) = balloon_text(&Notice::Up { secs: 42.0 });
        assert_eq!(body, i18n::toast_restored(42.0));
        let (title, body) =
            balloon_text(&Notice::Down { status: Status::LanDown, note: "no reply".into() });
        assert_eq!(title, Status::LanDown.headline());
        assert_eq!(body, "no reply");
    }
}
