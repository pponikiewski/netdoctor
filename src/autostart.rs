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

/// True when autostart points at an executable that no longer exists.
pub fn is_stale() -> bool {
    let Some(cmd) = current_command() else {
        return false;
    };
    let path = cmd.trim_start_matches('"').split('"').next().unwrap_or("").to_string();
    !path.is_empty() && !std::path::Path::new(&path).exists()
}

pub fn enable() -> Result<String> {
    let cmd = command_line()?;
    winreg::write_string(Root::CurrentUser, RUN_KEY, VALUE, &cmd)?;
    Ok(crate::i18n::auto_enabled().into())
}

pub fn disable() -> Result<String> {
    winreg::delete_value(Root::CurrentUser, RUN_KEY, VALUE)?;
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
    let wide = |s: &str| -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    };
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());

    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
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
