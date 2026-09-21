//! Minimal registry access.
//!
//! The Python prototype shelled out to `reg.exe` and parsed its output, which
//! meant a missing value and a failed command looked the same. These calls
//! return typed results, so "the value is absent" is distinguishable from
//! "access denied" — and that difference decides whether a tweak reports
//! "not set" or "needs administrator".

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_DWORD,
    REG_SZ,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    LocalMachine,
    CurrentUser,
}

impl Root {
    fn hkey(&self) -> HKEY {
        match self {
            Root::LocalMachine => HKEY_LOCAL_MACHINE,
            Root::CurrentUser => HKEY_CURRENT_USER,
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open(root: Root, path: &str, write: bool) -> Result<Key> {
    let access =
        if write { KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY } else { KEY_READ | KEY_WOW64_64KEY };
    let mut hkey = HKEY::default();
    let rc =
        unsafe { RegOpenKeyExW(root.hkey(), PCWSTR(wide(path).as_ptr()), 0, access, &mut hkey) };
    if rc != ERROR_SUCCESS {
        return Err(anyhow!("cannot open {path}: {}", describe(rc)));
    }
    Ok(Key(hkey))
}

/// `Ok(None)` when the key itself is not there.
///
/// A missing key is not a failure to read. Several tweaks write a policy key
/// that does not exist until someone sets the policy — which is why `apply`
/// creates it — so "the key is absent" belongs with "the value is absent":
/// both mean Windows is running on its own default. Only a key that exists
/// and still would not open, which in practice means access denied, is an
/// error, and the caller has to be able to say so rather than offer to change
/// something it could not read.
fn open_for_read(root: Root, path: &str) -> Result<Option<Key>> {
    let mut hkey = HKEY::default();
    let rc = unsafe {
        RegOpenKeyExW(
            root.hkey(),
            PCWSTR(wide(path).as_ptr()),
            0,
            KEY_READ | KEY_WOW64_64KEY,
            &mut hkey,
        )
    };
    match rc {
        ERROR_SUCCESS => Ok(Some(Key(hkey))),
        e if e == ERROR_FILE_NOT_FOUND || e == ERROR_PATH_NOT_FOUND => Ok(None),
        e => Err(anyhow!("cannot open {path}: {}", describe(e))),
    }
}

/// Creates a key if it is missing, and does nothing if it is already there.
/// Policy keys such as the Delivery Optimization one are absent on a machine
/// that has never had the policy set, so a tweak that writes one has to make
/// it first.
pub fn create_key(root: Root, path: &str) -> Result<()> {
    use windows::Win32::System::Registry::{RegCreateKeyExW, REG_OPTION_NON_VOLATILE};
    let mut hkey = HKEY::default();
    let rc = unsafe {
        RegCreateKeyExW(
            root.hkey(),
            PCWSTR(wide(path).as_ptr()),
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY,
            None,
            &mut hkey,
            None,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(anyhow!("cannot create {path}: {}", describe(rc)));
    }
    let _ = Key(hkey);
    Ok(())
}

/// `Ok(None)` means the value simply is not there, which is a normal state.
///
/// A value of the wrong type is an error, not a number: `RegQueryValueExW`
/// will happily copy the first four bytes of a `REG_BINARY` into our `u32`
/// and report success, and a tweak deciding whether the machine is already
/// configured off four bytes of something else is worse than one that admits
/// it cannot tell.
pub fn read_dword(root: Root, path: &str, name: &str) -> Result<Option<u32>> {
    let Some(key) = open_for_read(root, path)? else { return Ok(None) };
    let mut data: u32 = 0;
    let mut size: u32 = 4;
    let mut kind = REG_DWORD;
    let rc = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(wide(name).as_ptr()),
            None,
            Some(&mut kind),
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut size),
        )
    };
    match rc {
        ERROR_SUCCESS if kind != REG_DWORD => {
            Err(anyhow!("{name} is not a DWORD (type {})", kind.0))
        }
        ERROR_SUCCESS if size != 4 => Err(anyhow!("{name} is {size} bytes, not 4")),
        ERROR_SUCCESS => Ok(Some(data)),
        e if e == ERROR_FILE_NOT_FOUND => Ok(None),
        e => Err(anyhow!("cannot read {name}: {}", describe(e))),
    }
}

pub fn write_dword(root: Root, path: &str, name: &str, value: u32) -> Result<()> {
    let key = open(root, path, true)?;
    let bytes = value.to_le_bytes();
    let rc =
        unsafe { RegSetValueExW(key.0, PCWSTR(wide(name).as_ptr()), 0, REG_DWORD, Some(&bytes)) };
    if rc != ERROR_SUCCESS {
        return Err(anyhow!("cannot write {name}: {}", describe(rc)));
    }
    Ok(())
}

/// Reads `REG_SZ` or `REG_EXPAND_SZ`. Anything else is refused rather than
/// reinterpreted: a `REG_MULTI_SZ` read this way silently becomes its first
/// entry, and a `REG_BINARY` becomes noise that looks like text.
pub fn read_string(root: Root, path: &str, name: &str) -> Result<Option<String>> {
    use windows::Win32::System::Registry::REG_EXPAND_SZ;

    let key = open(root, path, false)?;
    let mut size: u32 = 0;
    let mut kind = REG_SZ;
    let rc = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(wide(name).as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    if rc == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if rc != ERROR_SUCCESS {
        return Err(anyhow!("cannot size {name}: {}", describe(rc)));
    }
    if kind != REG_SZ && kind != REG_EXPAND_SZ {
        return Err(anyhow!("{name} is not a string (type {})", kind.0));
    }
    let mut buf = vec![0u8; size as usize];
    let rc = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(wide(name).as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(anyhow!("cannot read {name}: {}", describe(rc)));
    }
    // The second call can report fewer bytes than the first reserved.
    buf.truncate(size as usize);
    let wide_chars: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|c| *c != 0)
        .collect();
    Ok(Some(String::from_utf16_lossy(&wide_chars)))
}

pub fn write_string(root: Root, path: &str, name: &str, value: &str) -> Result<()> {
    let key = open(root, path, true)?;
    let data = wide(value);
    let bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
    let rc = unsafe { RegSetValueExW(key.0, PCWSTR(wide(name).as_ptr()), 0, REG_SZ, Some(bytes)) };
    if rc != ERROR_SUCCESS {
        return Err(anyhow!("cannot write {name}: {}", describe(rc)));
    }
    Ok(())
}

/// Deleting an absent value counts as success — the caller wanted it gone.
pub fn delete_value(root: Root, path: &str, name: &str) -> Result<()> {
    let key = open(root, path, true)?;
    let rc = unsafe { RegDeleteValueW(key.0, PCWSTR(wide(name).as_ptr())) };
    match rc {
        ERROR_SUCCESS => Ok(()),
        e if e == ERROR_FILE_NOT_FOUND => Ok(()),
        e => Err(anyhow!("cannot delete {name}: {}", describe(e))),
    }
}

/// Enumerate subkeys of a path, returning their names.
///
/// A name longer than the buffer used to end the walk, so one oversized
/// sibling could hide every adapter after it and `adapter_class_key` would
/// report "no adapter" on a machine that has one. Only `ERROR_NO_MORE_ITEMS`
/// ends the loop now; a too-small buffer just skips that one entry.
pub fn subkeys(root: Root, path: &str) -> Vec<String> {
    use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS};
    use windows::Win32::System::Registry::RegEnumKeyExW;
    let Ok(key) = open(root, path, false) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut index = 0u32;
    loop {
        // 255 characters is the documented maximum for a key name.
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        let rc = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                windows::core::PWSTR(name.as_mut_ptr()),
                &mut len,
                None,
                windows::core::PWSTR::null(),
                None,
                None,
            )
        };
        match rc {
            ERROR_SUCCESS => out.push(String::from_utf16_lossy(&name[..len as usize])),
            e if e == ERROR_MORE_DATA => {}
            e if e == ERROR_NO_MORE_ITEMS => break,
            _ => break,
        }
        index += 1;
    }
    out
}

/// Enumerate the value names under a path. Adapter driver parameters are
/// listed this way: the option list of an advanced property is a key whose
/// *value names* are what gets written and whose data is the wording Device
/// Manager shows, so both halves are needed to pick an option by meaning.
pub fn value_names(root: Root, path: &str) -> Vec<String> {
    use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS};
    use windows::Win32::System::Registry::RegEnumValueW;
    let Ok(key) = open(root, path, false) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut index = 0u32;
    loop {
        let mut name = [0u16; 512];
        let mut len = name.len() as u32;
        let rc = unsafe {
            RegEnumValueW(
                key.0,
                index,
                windows::core::PWSTR(name.as_mut_ptr()),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        // Same reasoning as `subkeys`: skip what does not fit, stop only at
        // the real end of the list.
        match rc {
            ERROR_SUCCESS => out.push(String::from_utf16_lossy(&name[..len as usize])),
            e if e == ERROR_MORE_DATA => {}
            e if e == ERROR_NO_MORE_ITEMS => break,
            _ => break,
        }
        index += 1;
    }
    out
}

fn describe(e: WIN32_ERROR) -> String {
    match e.0 {
        5 => "access denied (needs administrator)".into(),
        2 => "not found".into(),
        other => format!("error {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CV: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

    #[test]
    fn reads_a_well_known_string() {
        let v = read_string(Root::LocalMachine, CV, "ProductName").unwrap();
        assert!(v.is_some(), "every Windows install has a ProductName");
        assert!(!v.unwrap().is_empty());
    }

    #[test]
    fn absent_value_is_none_not_an_error() {
        let v = read_dword(Root::LocalMachine, CV, "NetDoctorDefinitelyNotAValue").unwrap();
        assert!(v.is_none());
    }

    #[test]
    fn a_missing_key_reads_as_absent_rather_than_as_a_failure() {
        // This used to be an error, and the one caller that cared could not
        // tell it apart from access denied, so it treated both as "not set"
        // and offered to change a value it had never read. A key that is not
        // there is the same fact as a value that is not there: Windows is on
        // its own default. The tweaks that write policy keys create them.
        let v = read_dword(Root::LocalMachine, r"SOFTWARE\NetDoctorNope", "x").unwrap();
        assert!(v.is_none());
    }

    #[test]
    fn a_value_of_the_wrong_type_is_an_error_and_not_a_number() {
        // The case that makes "cannot read" reachable without an admin shell:
        // a REG_SZ where a DWORD is expected. RegQueryValueExW would happily
        // hand back the first four bytes of the string.
        // The same key the round-trip test uses, so the tests leave one stray
        // key behind between them rather than one each.
        let path = r"Software\NetDoctorTest";
        create_key(Root::CurrentUser, path).unwrap();
        write_string(Root::CurrentUser, path, "wrongtype", "not a number").unwrap();

        let read = read_dword(Root::CurrentUser, path, "wrongtype");
        assert!(read.is_err(), "four bytes of a string is not a reading: {read:?}");

        delete_value(Root::CurrentUser, path, "wrongtype").unwrap();
    }

    #[test]
    fn enumerating_subkeys_finds_network_adapters() {
        let keys = subkeys(
            Root::LocalMachine,
            r"SYSTEM\CurrentControlSet\Control\Class\{4d36e972-e325-11ce-bfc1-08002be10318}",
        );
        assert!(
            keys.iter().any(|k| k.chars().all(|c| c.is_ascii_digit())),
            "expected numbered adapter subkeys, got {keys:?}"
        );
    }

    #[test]
    fn round_trip_in_the_user_hive() {
        // HKCU is writable without elevation, so this exercises the write path
        // without needing admin in CI.
        let path = r"Software\NetDoctorTest";
        create_key(Root::CurrentUser, path).unwrap();

        write_dword(Root::CurrentUser, path, "probe", 42).unwrap();
        assert_eq!(read_dword(Root::CurrentUser, path, "probe").unwrap(), Some(42));
        write_string(Root::CurrentUser, path, "text", "hello").unwrap();
        assert_eq!(
            read_string(Root::CurrentUser, path, "text").unwrap(),
            Some("hello".to_string())
        );
        delete_value(Root::CurrentUser, path, "probe").unwrap();
        assert_eq!(read_dword(Root::CurrentUser, path, "probe").unwrap(), None);
        // Deleting twice is still fine.
        delete_value(Root::CurrentUser, path, "probe").unwrap();
        delete_value(Root::CurrentUser, path, "text").unwrap();
    }
}
