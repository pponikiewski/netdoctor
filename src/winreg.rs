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
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
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
    let access = if write {
        KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY
    } else {
        KEY_READ | KEY_WOW64_64KEY
    };
    let mut hkey = HKEY::default();
    let rc = unsafe {
        RegOpenKeyExW(root.hkey(), PCWSTR(wide(path).as_ptr()), 0, access, &mut hkey)
    };
    if rc != ERROR_SUCCESS {
        return Err(anyhow!("cannot open {path}: {}", describe(rc)));
    }
    Ok(Key(hkey))
}

/// `Ok(None)` means the value simply is not there, which is a normal state.
pub fn read_dword(root: Root, path: &str, name: &str) -> Result<Option<u32>> {
    let key = open(root, path, false)?;
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
        ERROR_SUCCESS => Ok(Some(data)),
        e if e == ERROR_FILE_NOT_FOUND => Ok(None),
        e => Err(anyhow!("cannot read {name}: {}", describe(e))),
    }
}

pub fn write_dword(root: Root, path: &str, name: &str, value: u32) -> Result<()> {
    let key = open(root, path, true)?;
    let bytes = value.to_le_bytes();
    let rc = unsafe {
        RegSetValueExW(key.0, PCWSTR(wide(name).as_ptr()), 0, REG_DWORD, Some(&bytes))
    };
    if rc != ERROR_SUCCESS {
        return Err(anyhow!("cannot write {name}: {}", describe(rc)));
    }
    Ok(())
}

pub fn read_string(root: Root, path: &str, name: &str) -> Result<Option<String>> {
    let key = open(root, path, false)?;
    let mut size: u32 = 0;
    let rc = unsafe {
        RegQueryValueExW(key.0, PCWSTR(wide(name).as_ptr()), None, None, None, Some(&mut size))
    };
    if rc == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if rc != ERROR_SUCCESS {
        return Err(anyhow!("cannot size {name}: {}", describe(rc)));
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
    let bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2)
    };
    let rc =
        unsafe { RegSetValueExW(key.0, PCWSTR(wide(name).as_ptr()), 0, REG_SZ, Some(bytes)) };
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
pub fn subkeys(root: Root, path: &str) -> Vec<String> {
    use windows::Win32::System::Registry::RegEnumKeyExW;
    let Ok(key) = open(root, path, false) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut index = 0u32;
    loop {
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
        if rc != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
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
    fn missing_key_is_an_error() {
        assert!(read_dword(Root::LocalMachine, r"SOFTWARE\NetDoctorNope", "x").is_err());
    }

    #[test]
    fn enumerating_subkeys_finds_network_adapters() {
        let keys = subkeys(
            Root::LocalMachine,
            r"SYSTEM\CurrentControlSet\Control\Class\{4d36e972-e325-11ce-bfc1-08002be10318}",
        );
        assert!(keys.iter().any(|k| k.chars().all(|c| c.is_ascii_digit())),
                "expected numbered adapter subkeys, got {keys:?}");
    }

    #[test]
    fn round_trip_in_the_user_hive() {
        // HKCU is writable without elevation, so this exercises the write path
        // without needing admin in CI.
        let path = r"Software\NetDoctorTest";
        let key = unsafe {
            let mut h = HKEY::default();
            windows::Win32::System::Registry::RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(wide(path).as_ptr()),
                0,
                None,
                windows::Win32::System::Registry::REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_SET_VALUE,
                None,
                &mut h,
                None,
            );
            Key(h)
        };
        drop(key);

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
