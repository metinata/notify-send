//! How notify-send identifies apps and notifications to Windows. The
//! integration tests share this module, so it has no tests of its own; they
//! are in `src/app.rs`.

/// The app name without `-a`, as in libnotify.
pub const DEFAULT_NAME: &str = "notify-send";

/// The AUMID of `DEFAULT_NAME`, and the prefix of the others.
const AUMID: &str = "metinata.notify-send";

/// The group of every notification notify-send shows. The tag is the
/// notification's ID, so that `-r` replaces the one with the same ID.
pub const GROUP: &str = "notify-send";

/// The AUMID for an app name. Never change this: Windows keeps the user's
/// notification settings for each app (on or off, sound, priority) by AUMID.
pub fn aumid(name: &str) -> String {
    if name == DEFAULT_NAME {
        AUMID.to_owned()
    } else {
        // An AUMID may not contain spaces and is at most 128 characters, so
        // names are hashed instead of used as they are.
        format!("{AUMID}.{:016x}", fnv1a(name.as_bytes()))
    }
}

/// 64-bit FNV-1a: small, and stable across versions and platforms.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
