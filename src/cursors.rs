//! Reads and writes HKCU\Control Panel\Cursors and tells Windows to reload the cursors.

use crate::scheme::ROLES;
use serde::{Deserialize, Serialize};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_SETCURSORS, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SystemParametersInfoW,
};

pub const CURSORS_KEY: &str = r"Control Panel\Cursors";
pub const SCHEMES_KEY: &str = r"Control Panel\Cursors\Schemes";
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn opt_wide(s: Option<&str>) -> Option<Vec<u16>> {
    s.map(wide)
}

/// Reads a REG_SZ / REG_EXPAND_SZ value without expanding it. `None` name = the default value.
pub fn get_string(sub: &str, name: Option<&str>) -> Option<String> {
    let sub_w = wide(sub);
    let name_w = opt_wide(name);
    let name_p = name_w.as_ref().map_or(null(), |v| v.as_ptr());
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    unsafe {
        let mut size: u32 = 0;
        if RegGetValueW(HKEY_CURRENT_USER, sub_w.as_ptr(), name_p, flags, null_mut(), null_mut(), &mut size)
            != ERROR_SUCCESS
        {
            return None;
        }
        let mut buf = vec![0u16; (size as usize / 2) + 1];
        let mut size = (buf.len() * 2) as u32;
        if RegGetValueW(
            HKEY_CURRENT_USER,
            sub_w.as_ptr(),
            name_p,
            flags,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

pub fn get_dword(sub: &str, name: &str) -> Option<u32> {
    let sub_w = wide(sub);
    let name_w = wide(name);
    let mut val: u32 = 0;
    let mut size = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            sub_w.as_ptr(),
            name_w.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut val as *mut u32).cast(),
            &mut size,
        )
    };
    (r == ERROR_SUCCESS).then_some(val)
}

fn with_key<T>(sub: &str, f: impl FnOnce(HKEY) -> T) -> Result<T, String> {
    let sub_w = wide(sub);
    let mut key: HKEY = null_mut();
    let r = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            sub_w.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_QUERY_VALUE,
            null(),
            &mut key,
            null_mut(),
        )
    };
    if r != ERROR_SUCCESS {
        return Err(format!("cannot open HKCU\\{sub} (error {r})"));
    }
    let out = f(key);
    unsafe { RegCloseKey(key) };
    Ok(out)
}

pub fn set_string(sub: &str, name: Option<&str>, value: &str, expand: bool) -> Result<(), String> {
    let name_w = opt_wide(name);
    let data = wide(value);
    let r = with_key(sub, |key| unsafe {
        RegSetValueExW(
            key,
            name_w.as_ref().map_or(null(), |v| v.as_ptr()),
            0,
            if expand { REG_EXPAND_SZ } else { REG_SZ },
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    })?;
    if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("cannot write {name:?} (error {r})")) }
}

pub fn set_dword(sub: &str, name: &str, value: u32) -> Result<(), String> {
    let name_w = wide(name);
    let r = with_key(sub, |key| unsafe {
        RegSetValueExW(key, name_w.as_ptr(), 0, REG_DWORD, (&value as *const u32).cast(), 4)
    })?;
    if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("cannot write {name} (error {r})")) }
}

pub fn delete_value(sub: &str, name: &str) {
    let sub_w = wide(sub);
    let name_w = wide(name);
    unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, sub_w.as_ptr(), name_w.as_ptr()) };
}

fn reload_cursors() {
    unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, null_mut(), SPIF_UPDATEINIFILE | SPIF_SENDCHANGE) };
}

/// The user's cursor settings from before CursorDrop touched anything.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Backup {
    pub scheme_name: Option<String>,
    pub scheme_source: Option<u32>,
    pub values: Vec<(String, Option<String>)>,
}

pub fn take_backup() -> Backup {
    Backup {
        scheme_name: get_string(CURSORS_KEY, None),
        scheme_source: get_dword(CURSORS_KEY, "Scheme Source"),
        values: ROLES.iter().map(|r| (r.to_string(), get_string(CURSORS_KEY, Some(r)))).collect(),
    }
}

pub fn restore(b: &Backup) -> Result<(), String> {
    for (name, val) in &b.values {
        match val {
            Some(v) => set_string(CURSORS_KEY, Some(name), v, true)?,
            None => delete_value(CURSORS_KEY, name),
        }
    }
    set_string(CURSORS_KEY, None, b.scheme_name.as_deref().unwrap_or(""), false)?;
    match b.scheme_source {
        Some(v) => set_dword(CURSORS_KEY, "Scheme Source", v)?,
        None => delete_value(CURSORS_KEY, "Scheme Source"),
    }
    reload_cursors();
    Ok(())
}

/// Applies a scheme: `paths` holds one absolute path (or None = Windows built-in) per role.
pub fn apply(name: &str, paths: &[Option<String>]) -> Result<(), String> {
    for (i, role) in ROLES.iter().enumerate() {
        let v = paths.get(i).cloned().flatten().unwrap_or_default();
        set_string(CURSORS_KEY, Some(role), &v, true)?;
    }
    set_string(CURSORS_KEY, None, name, false)?;
    set_dword(CURSORS_KEY, "Scheme Source", 1)?; // 1 = user scheme
    // Also list it in Mouse Properties > Pointers so it can be picked there.
    let list: Vec<String> = (0..ROLES.len()).map(|i| paths.get(i).cloned().flatten().unwrap_or_default()).collect();
    let _ = set_string(SCHEMES_KEY, Some(name), &list.join(","), true);
    reload_cursors();
    Ok(())
}

pub fn unregister_scheme(name: &str) {
    delete_value(SCHEMES_KEY, name);
}

pub fn current_scheme_name() -> Option<String> {
    get_string(CURSORS_KEY, None)
}
