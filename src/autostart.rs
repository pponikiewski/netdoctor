//! Start with Windows, via the per-user Run key.
//!
//! No elevation needed: the monitor does not require admin, only applying
//! tweaks does. An unelevated autostart is enough to keep catching dropouts,
//! which is the part that has to be running *before* the problem happens.

use anyhow::{anyhow, Result};

use crate::winreg::{self, Root};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "NetDoctor";

fn command_line() -> Result<String> {
    let exe = std::env::current_exe()?;
    Ok(format!("\"{}\" --minimised", exe.display()))
}

pub fn is_enabled() -> bool {
    matches!(winreg::read_string(Root::CurrentUser, RUN_KEY, VALUE), Ok(Some(_)))
}

/// The registered command, if any — shown in the UI so a stale entry that
/// points at a moved executable is visible rather than mysterious.
pub fn current_command() -> Option<String> {
    winreg::read_string(Root::CurrentUser, RUN_KEY, VALUE).ok().flatten()
}

/// The executable an autostart command line points at.
///
/// We always write the path in quotes, but an entry left by an older build or
/// edited by hand may not be quoted — and splitting an unquoted line on `"`
/// used to hand back the arguments as part of the path, which then "did not
/// exist" and had the UI report a working autostart as broken.
fn executable_in(cmd: &str) -> &str {
    let cmd = cmd.trim();
    match cmd.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        // Unquoted: everything up to the first switch, which is the only
        // shape we ever write without quotes.
        None => cmd.split(" --").next().unwrap_or(cmd).trim_end(),
    }
}

/// True when autostart points at an executable that no longer exists.
pub fn is_stale() -> bool {
    let Some(cmd) = current_command() else {
        return false;
    };
    let path = executable_in(&cmd);
    !path.is_empty() && !std::path::Path::new(path).exists()
}

pub fn enable() -> Result<String> {
    let cmd = command_line()?;
    // A profile that has never had a startup entry has no Run key at all, and
    // opening a key that is not there fails. Creating it first is a no-op on
    // the machines that do have one, which is nearly all of them — but "Start
    // with Windows" reporting "not found" on a fresh profile is not a failure
    // anyone can act on.
    winreg::create_key(Root::CurrentUser, RUN_KEY)?;
    winreg::write_string(Root::CurrentUser, RUN_KEY, VALUE, &cmd)?;
    Ok(crate::i18n::auto_enabled().into())
}

pub fn disable() -> Result<String> {
    // No Run key at all is already the state the caller is asking for, and an
    // absent value is treated the same way one level down. Neither is worth
    // reporting as a failure to switch something off.
    if is_enabled() {
        winreg::delete_value(Root::CurrentUser, RUN_KEY, VALUE)?;
    }
    Ok(crate::i18n::auto_disabled().into())
}

pub fn set(enabled: bool) -> Result<String> {
    if enabled {
        enable()
    } else {
        disable()
    }
}

/// Relaunch elevated through the shell's "runas" verb and ask the caller to
/// exit. Returns an error if the user dismissed the UAC prompt.
pub fn relaunch_elevated() -> Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let exe = std::env::current_exe()?;
    let wide =
        |s: &str| -> Vec<u16> { OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect() };
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    // The elevated copy starts while this one is still closing, so it has to
    // wait for the single-instance name like an updated copy does.
    let params = wide(crate::update::AFTER_UPDATE_FLAG);

    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(params.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns >32 on success.
    if result.0 as isize <= 32 {
        return Err(anyhow!(crate::i18n::auto_elevation_declined()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_the_path_and_asks_for_minimised() {
        let cmd = command_line().unwrap();
        assert!(cmd.starts_with('"'), "an unquoted path breaks on spaces: {cmd}");
        assert!(cmd.ends_with("--minimised"));
    }

    #[test]
    fn executable_is_found_quoted_or_not() {
        assert_eq!(
            executable_in(r#""C:\Program Files\NetDoctor\netdoctor.exe" --minimised"#),
            r"C:\Program Files\NetDoctor\netdoctor.exe"
        );
        // The shape an older build or a hand edit leaves behind.
        assert_eq!(executable_in(r"C:\Tools\netdoctor.exe --minimised"), r"C:\Tools\netdoctor.exe");
        assert_eq!(executable_in(r"C:\Tools\netdoctor.exe"), r"C:\Tools\netdoctor.exe");
    }

    #[test]
    fn enable_then_disable_round_trips() {
        let was_enabled = is_enabled();

        enable().unwrap();
        assert!(is_enabled());
        assert!(current_command().unwrap().contains("--minimised"));
        // The entry we just wrote points at this test binary, which exists.
        assert!(!is_stale());

        disable().unwrap();
        assert!(!is_enabled());
        // Disabling twice is not an error.
        disable().unwrap();

        // Leave the machine as we found it.
        if was_enabled {
            enable().unwrap();
        }
    }
}
