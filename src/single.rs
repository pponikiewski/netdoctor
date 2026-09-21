//! One instance at a time.
//!
//! Two copies running is not a cosmetic problem. Both sweep the same targets
//! once a second and both write the rows into the same `history.db`, and
//! `Store::stats` reads the distance between consecutive rows as jitter — so
//! the second instance does not merely duplicate the history, it reports a
//! calmer line than the one being measured. See the test in [`crate::store`]
//! that reproduces exactly that.
//!
//! The realistic way in is not deliberate: autostart launches a copy with
//! `--minimised`, the window is nowhere to be seen, and the user clicks the
//! shortcut. So the second launch raises the first one's window rather than
//! saying no and leaving them with nothing.

#[cfg(windows)]
mod imp {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, BOOL, ERROR_ALREADY_EXISTS, FALSE, HANDLE, HWND, LPARAM, TRUE,
    };
    use windows::Win32::System::Threading::{
        CreateMutexW, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, SetForegroundWindow,
        ShowWindow, SW_RESTORE,
    };

    /// Per session, not machine-wide. The history this guards lives under the
    /// user's own `AppData`, so two people signed in at once are two separate
    /// databases and have every right to a copy each. `Global\` would also
    /// need a privilege an ordinary account does not have.
    const NAME: &str = "Local\\netdoctor-single-instance";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Held for as long as this process is the one instance. Dropping it —
    /// including on the way out of `main` — releases the name for the next
    /// launch.
    pub struct Guard(HANDLE);

    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// `None` when another copy already holds the name.
    ///
    /// The handle is kept even in that case until the function returns: the
    /// mutex exists either way, and closing it is what makes the name
    /// available again, so the loser has to close its own handle rather than
    /// leak a reference to the winner's.
    pub fn acquire() -> Option<Guard> {
        acquire_named(NAME)
    }

    /// The name is a parameter so a test can take one of its own. Using the
    /// real name in a test would make `cargo test` fail on any machine where
    /// the app happens to be running, which is most of them, most of the time.
    pub(super) fn acquire_named(name: &str) -> Option<Guard> {
        let name = wide(name);
        let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) }.ok()?;
        // Documented order: GetLastError is read after a successful create,
        // because "created" and "opened an existing one" both succeed.
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let guard = Guard(handle);
        (!already).then_some(guard)
    }

    struct Search {
        exe: Vec<u16>,
        found: HWND,
    }

    fn exe_of(pid: u32) -> Option<Vec<u16>> {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 260];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_FORMAT(0),
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(process);
            ok.ok()?;
            Some(buf[..len as usize].to_vec())
        }
    }

    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
        let search = &mut *(param.0 as *mut Search);

        // Not filtered on visibility: the case this exists for is autostart's
        // `--minimised` copy, whose window is built with `with_visible(false)`
        // and so is hidden rather than merely small. Filtered on having a
        // title instead, which is what separates the real window from the
        // message-only and helper windows winit keeps alongside it.
        if GetWindowTextLengthW(hwnd) == 0 {
            return TRUE;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return TRUE;
        }
        // Matched on the executable rather than on the window title: the
        // title carries the version and a translated tagline, and a text
        // editor with this project open would match it.
        // Compared case-insensitively over UTF-16: Windows paths are, and the
        // same binary launched from a shortcut and from a shell can differ in
        // the case of the drive letter alone.
        let same = |a: &[u16], b: &[u16]| {
            a.len() == b.len()
                && a.iter().zip(b).all(|(x, y)| {
                    char::from_u32(*x as u32)
                        .zip(char::from_u32(*y as u32))
                        .map(|(x, y)| x.eq_ignore_ascii_case(&y))
                        .unwrap_or(x == y)
                })
        };
        match exe_of(pid) {
            Some(exe) if same(&exe, &search.exe) => {
                search.found = hwnd;
                FALSE
            }
            _ => TRUE,
        }
    }

    /// Brings the running copy's window to the front, showing it first when
    /// it is hidden or minimised. `false` when no such window was found.
    pub fn raise_existing_window() -> bool {
        let Ok(exe) = std::env::current_exe() else { return false };
        let exe: Vec<u16> = {
            use std::os::windows::ffi::OsStrExt;
            exe.as_os_str().encode_wide().collect()
        };
        let mut search = Search { exe, found: HWND::default() };

        unsafe {
            let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
            if search.found.0.is_null() {
                return false;
            }
            // Unconditionally: `SW_RESTORE` shows a hidden window as well as
            // un-minimising one, and both are states the running copy can be
            // in when somebody goes looking for it.
            let _ = ShowWindow(search.found, SW_RESTORE);
            SetForegroundWindow(search.found).as_bool()
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct Guard;

    pub fn acquire() -> Option<Guard> {
        Some(Guard)
    }

    pub fn raise_existing_window() -> bool {
        false
    }
}

pub use imp::{acquire, raise_existing_window};

#[cfg(all(test, windows))]
mod tests {
    use super::imp::acquire_named;

    #[test]
    fn the_name_is_held_by_one_holder_at_a_time() {
        // Its own name, not the app's: a developer with netdoctor open would
        // otherwise watch this test fail for the very reason it passes.
        let name = format!("Local\\netdoctor-test-{}", std::process::id());

        let first = acquire_named(&name).expect("nothing else holds this name");
        assert!(
            acquire_named(&name).is_none(),
            "a second acquire has to fail, whichever process it comes from"
        );

        drop(first);
        let again = acquire_named(&name).expect("closing the handle releases the name");
        drop(again);
    }
}
