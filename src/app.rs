//! The app a notification comes from. Windows shows each notification under an
//! AppUserModelID (AUMID) and takes the app's name and icon from that AUMID's
//! registry key, which an unpackaged program can create for the current user:
//! `HKCU\Software\Classes\AppUserModelId\<AUMID>`, with `DisplayName` and
//! `IconUri`. No Start menu shortcut is needed.
//!
//! Every app name (`-a`) gets an AUMID of its own (see `src/identity.rs`), so
//! notifications are grouped by app, show its name, and can be turned off per
//! app in Settings → System → Notifications, as on Linux.

use std::fs;
use std::path::{Path, PathBuf};

use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{HSTRING, Result};

use crate::identity::{self, fnv1a};

const KEY: &str = r"Software\Classes\AppUserModelId";

/// The icon of apps that have none of their own (`-n`).
const ICON: &[u8] = include_bytes!("../assets/notify-send.png");

#[derive(Debug, PartialEq)]
pub struct App {
    pub aumid: String,
    pub name: String,
    /// An image file to use as the app's icon; notify-send's own without one.
    pub icon: Option<PathBuf>,
}

impl App {
    pub fn new(name: &str, icon: Option<PathBuf>) -> Self {
        Self {
            aumid: identity::aumid(name),
            name: name.to_owned(),
            icon,
        }
    }
}

/// Registers the app's name and icon for its AUMID. This must happen before
/// the AUMID's first notification: for an AUMID it does not know, Windows
/// shows the AUMID itself as the name and keeps doing so. Values are only
/// written when they change.
pub fn register(app: &App) -> Result<()> {
    let key = format!(r"{KEY}\{}", app.aumid);
    set_string(&key, "DisplayName", &display_name(&app.name))?;
    // If notify-send's icon cannot be written (LOCALAPPDATA is not set, say),
    // the icon registered before stays.
    match app.icon.clone().or_else(default_icon) {
        Some(icon) => set_string(&key, "IconUri", &icon.to_string_lossy()),
        None => Ok(()),
    }
}

/// The `DisplayName` for an app name. Windows reads one that starts with `@`
/// as a reference to a string in a module (`@module,-id`) and shows that
/// string instead; a zero-width space in front keeps the name as it is.
fn display_name(name: &str) -> String {
    if name.starts_with('@') {
        format!("\u{200b}{name}")
    } else {
        name.to_owned()
    }
}

fn get_string(key: &str, name: &str) -> Option<String> {
    let (key, name) = (HSTRING::from(key), HSTRING::from(name));
    let mut bytes = 0u32;
    // SAFETY: with no buffer, RegGetValueW only stores the size the value needs.
    let size = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&raw mut bytes),
        )
    };
    size.ok().ok()?;
    let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
    // SAFETY: `buffer` holds `bytes` bytes; RegGetValueW writes at most that
    // many, NUL-terminated, and updates `bytes` to what it wrote.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut bytes),
        )
    };
    read.ok().ok()?;
    let chars = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16(&buffer[..chars]).ok()
}

/// Sets a string value, creating the key if needed.
fn set_string(key: &str, name: &str, value: &str) -> Result<()> {
    if get_string(key, name).as_deref() == Some(value) {
        return Ok(());
    }
    let data: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let bytes = u32::try_from(data.len() * 2).expect("registry string fits in u32");
    // SAFETY: `data` is a NUL-terminated UTF-16 string of `bytes` bytes, and
    // the key and value names are NUL-terminated HSTRINGs.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(name),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            bytes,
        )
    }
    .ok()
}

/// notify-send's icon as a file, which `IconUri` needs. It is written to
/// `%LOCALAPPDATA%\notify-send` under a name that changes with its contents:
/// Windows picks up a new `IconUri` at once, but may keep showing a changed
/// file under the same name. Icons of other versions stay, since apps
/// registered by them still point to them. `None` if the file cannot be
/// written.
fn default_icon() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("notify-send");
    let path = dir.join(format!("icon-{:08x}.png", fnv1a(ICON) as u32));
    if fs::read(&path).is_ok_and(|bytes| bytes == ICON) {
        return Some(path);
    }
    fs::create_dir_all(&dir).ok()?;
    write_atomically(&path, ICON).ok()?;
    Some(path)
}

/// Writes a temporary file and moves it into place, so that a notify-send
/// running at the same time never sees half a file.
fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temp, contents)
        .and_then(|()| fs::rename(&temp, path))
        .inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{DEFAULT_NAME, aumid};

    #[test]
    fn the_default_app_has_a_readable_aumid() {
        assert_eq!(aumid(DEFAULT_NAME), "metinata.notify-send");
    }

    #[test]
    fn aumids_never_change() {
        // Users' notification settings are stored per AUMID: these values
        // must stay the same in every version.
        assert_eq!(aumid("Backup"), "metinata.notify-send.d05190d6b6b60a1f");
        assert_eq!(
            aumid("Yedekleme ✓"),
            "metinata.notify-send.123358d77d4ae96b"
        );
        assert_ne!(aumid("backup"), aumid("Backup"));
    }

    #[test]
    fn fnv1a_matches_the_reference() {
        // Test vectors from the FNV reference implementation.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn display_names_never_start_with_an_at_sign() {
        assert_eq!(display_name("Backup"), "Backup");
        assert_eq!(display_name("@myorg/build"), "\u{200b}@myorg/build");
        assert_eq!(display_name("me@home"), "me@home");
    }
}
