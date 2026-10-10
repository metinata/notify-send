//! `-h TYPE:NAME:VALUE` hints: parsed and checked like libnotify's notify-send
//! does, then mapped onto Windows where there is a counterpart.
//!
//! One difference: libnotify parses `variant` values as GVariant text and
//! rejects text it cannot parse. Here they are kept as written, unchecked, and
//! ignored, since no hint Windows has a counterpart for needs one.

use crate::cli::{C_WHITESPACE, Urgency};

/// A hint's value, in the type its `TYPE` names.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Boolean(bool),
    Int(i32),
    /// As written: no hint that Windows has a counterpart for is a double.
    Double(String),
    String(String),
    Byte(u8),
    /// GVariant text, as written.
    Variant(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub name: String,
    pub value: Value,
}

/// Parses one `TYPE:NAME:VALUE` hint. The type is case-insensitive, and the
/// value may contain colons. Numbers are read the way libnotify reads them:
/// `int` and `double` must start with a digit, `byte` takes decimal or `0x`
/// hex up to 255, and `boolean` is true for `true` or a non-zero number.
pub fn parse(hint: &str) -> Result<Hint, String> {
    let mut parts = hint.splitn(3, ':');
    let (Some(kind), Some(name), Some(value)) = (parts.next(), parts.next(), parts.next()) else {
        return Err("Invalid hint syntax specified. Use TYPE:NAME:VALUE.".into());
    };
    let unparsable =
        || format!("Value \"{value}\" of hint \"{name}\" could not be parsed as type \"{kind}\".");
    let value = match kind.to_ascii_lowercase().as_str() {
        "string" => Value::String(value.into()),
        // Truncated to 32 bits as libnotify does, so this cannot fail.
        "int" if starts_with_digit(value) => Value::Int(strtoull(value, 10) as i32),
        "double" if starts_with_digit(value) => Value::Double(value.into()),
        "int" | "double" => return Err(unparsable()),
        "byte" => {
            let base = if value.starts_with("0x") { 16 } else { 10 };
            let byte = u8::try_from(strtoull(value, base) as i32).map_err(|_| unparsable())?;
            Value::Byte(byte)
        }
        "boolean" => Value::Boolean(
            value.eq_ignore_ascii_case("true")
                || (starts_with_digit(value) && strtoull(value, 10) != 0),
        ),
        "variant" => Value::Variant(value.into()),
        _ => {
            return Err(format!(
                "Invalid hint type \"{kind}\". Valid types are boolean, int, double, string, byte and variant."
            ));
        }
    };
    Ok(Hint {
        name: name.into(),
        value,
    })
}

/// What the hints change. Hints are applied in order, so a later one wins.
#[derive(Debug, Default, PartialEq)]
pub struct Effects {
    /// `urgency` (0 low, 1 normal, 2 critical), which overrides `-u`.
    pub urgency: Option<Urgency>,
    /// `transient`, which overrides `-e`, also to turn it off.
    pub transient: Option<bool>,
    /// `suppress-sound`.
    pub suppress_sound: bool,
    /// `sound-name`, a freedesktop sound name or a Windows sound.
    pub sound_name: Option<String>,
    /// `image-path` (or the older `image_path`), which overrides `-i`.
    pub image_path: Option<String>,
    /// `value`, a percentage that some Linux notification servers show as a
    /// progress bar, as Windows does.
    pub progress: Option<u8>,
}

/// Collects what the hints Windows has a counterpart for change. Other hints
/// (`category`, `desktop-entry`, `resident`, `x`, `y`, ...) are accepted and
/// ignored, as a notification server ignores hints it does not support.
pub fn effects(hints: &[Hint]) -> Effects {
    let mut effects = Effects::default();
    for hint in hints {
        let value = &hint.value;
        match hint.name.as_str() {
            "urgency" => {
                if let Some(level) = number(value) {
                    effects.urgency = match level {
                        0 => Some(Urgency::Low),
                        1 => Some(Urgency::Normal),
                        2.. => Some(Urgency::Critical),
                        _ => effects.urgency,
                    };
                }
            }
            "transient" => effects.transient = truth(value).or(effects.transient),
            "suppress-sound" => {
                effects.suppress_sound = truth(value).unwrap_or(effects.suppress_sound);
            }
            "sound-name" => {
                if let Value::String(name) = value {
                    effects.sound_name = Some(name.clone());
                }
            }
            "image-path" | "image_path" => {
                if let Value::String(path) = value {
                    effects.image_path = Some(path.clone());
                }
            }
            "value" => {
                if let Some(percent) = number(value) {
                    effects.progress = Some(percent.clamp(0, 100) as u8);
                }
            }
            _ => {}
        }
    }
    effects
}

/// The Windows sound for a `sound-name` hint, as a toast `<audio src>`: a few
/// freedesktop names have a close match, and Windows names work as they are
/// (`ms-winsoundevent:Notification.Mail`, or just `Mail`). `None` keeps the
/// default sound.
pub fn windows_sound(name: &str) -> Option<String> {
    if let Some(event) = name.strip_prefix("ms-winsoundevent:") {
        return Some(format!("ms-winsoundevent:{event}"));
    }
    let event = match name {
        "message-new-instant" => "IM",
        "message-new-email" => "Mail",
        "alarm-clock-elapsed" => "Reminder",
        _ => ["Default", "IM", "Mail", "Reminder", "SMS"]
            .into_iter()
            .find(|event| event.eq_ignore_ascii_case(name))?,
    };
    Some(format!("ms-winsoundevent:Notification.{event}"))
}

fn number(value: &Value) -> Option<i64> {
    match *value {
        Value::Int(n) => Some(n.into()),
        Value::Byte(n) => Some(n.into()),
        _ => None,
    }
}

fn truth(value: &Value) -> Option<bool> {
    match value {
        Value::Boolean(b) => Some(*b),
        other => number(other).map(|n| n != 0),
    }
}

fn starts_with_digit(text: &str) -> bool {
    text.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

/// C's `strtoull`: leading whitespace and a sign are allowed (`-1` wraps
/// around), base 16 allows a `0x` prefix, and parsing stops at the first
/// character that is not a digit. Overflow gives `u64::MAX`.
fn strtoull(text: &str, base: u32) -> u64 {
    let text = text.trim_start_matches(C_WHITESPACE);
    let (negative, text) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let text = match base {
        16 => text
            .strip_prefix("0x")
            .or_else(|| text.strip_prefix("0X"))
            .unwrap_or(text),
        _ => text,
    };
    let mut value = Some(0u64);
    for digit in text.chars().map_while(|c| c.to_digit(base)) {
        value = value
            .and_then(|v| v.checked_mul(base.into()))
            .and_then(|v| v.checked_add(digit.into()));
    }
    match value {
        Some(v) if negative => v.wrapping_neg(),
        Some(v) => v,
        None => u64::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(hint: &str) -> Value {
        parse(hint).unwrap().value
    }

    #[test]
    fn syntax() {
        let hint = parse("string:sound-name:message-new-instant").unwrap();
        assert_eq!(hint.name, "sound-name");
        assert_eq!(hint.value, Value::String("message-new-instant".into()));
        // The value may contain colons.
        assert_eq!(
            value("string:image-path:C:/icons/a.png"),
            Value::String("C:/icons/a.png".into())
        );
        assert_eq!(value("STRING:x:"), Value::String(String::new()));
        for bad in ["", "string", "string:name"] {
            assert_eq!(
                parse(bad),
                Err("Invalid hint syntax specified. Use TYPE:NAME:VALUE.".into())
            );
        }
        assert!(
            parse("float:x:1")
                .unwrap_err()
                .contains("Invalid hint type")
        );
    }

    #[test]
    fn integers() {
        assert_eq!(value("int:value:42"), Value::Int(42));
        assert_eq!(value("int:value:42abc"), Value::Int(42));
        assert_eq!(value("int:value:4294967295"), Value::Int(-1));
        let err = parse("int:value:-5").unwrap_err();
        assert_eq!(
            err,
            "Value \"-5\" of hint \"value\" could not be parsed as type \"int\"."
        );
        assert!(parse("int:value:abc").is_err());
    }

    #[test]
    fn doubles_must_start_with_a_digit() {
        assert_eq!(value("double:x:2.5"), Value::Double("2.5".into()));
        assert_eq!(value("double:x:1e3kg"), Value::Double("1e3kg".into()));
        assert!(parse("double:x:.5").is_err());
    }

    #[test]
    fn bytes() {
        assert_eq!(value("byte:urgency:2"), Value::Byte(2));
        assert_eq!(value("byte:urgency:0xff"), Value::Byte(255));
        // strtoull reads nothing from "abc", which makes 0, as in libnotify.
        assert_eq!(value("byte:urgency:abc"), Value::Byte(0));
        assert!(parse("byte:urgency:256").is_err());
        assert!(parse("byte:urgency:-1").is_err());
    }

    #[test]
    fn booleans() {
        assert_eq!(value("boolean:transient:true"), Value::Boolean(true));
        assert_eq!(value("boolean:transient:TRUE"), Value::Boolean(true));
        assert_eq!(value("boolean:transient:1"), Value::Boolean(true));
        assert_eq!(value("boolean:transient:0"), Value::Boolean(false));
        assert_eq!(value("boolean:transient:yes"), Value::Boolean(false));
    }

    #[test]
    fn variants_are_kept_as_text() {
        assert_eq!(
            value("variant:x:<int32 5>"),
            Value::Variant("<int32 5>".into())
        );
    }

    fn effects_of(hints: &[&str]) -> Effects {
        let hints: Vec<Hint> = hints.iter().map(|h| parse(h).unwrap()).collect();
        effects(&hints)
    }

    #[test]
    fn known_hints_have_effects() {
        let e = effects_of(&[
            "byte:urgency:2",
            "boolean:transient:true",
            "boolean:suppress-sound:1",
            "string:sound-name:message-new-email",
            "string:image-path:C:/a.png",
            "int:value:150",
        ]);
        assert_eq!(
            e,
            Effects {
                urgency: Some(Urgency::Critical),
                transient: Some(true),
                suppress_sound: true,
                sound_name: Some("message-new-email".into()),
                image_path: Some("C:/a.png".into()),
                progress: Some(100),
            }
        );
        assert_eq!(effects_of(&["byte:urgency:0"]).urgency, Some(Urgency::Low));
        assert_eq!(effects_of(&["int:value:40"]).progress, Some(40));
        assert_eq!(
            effects_of(&["boolean:transient:false"]).transient,
            Some(false)
        );
    }

    #[test]
    fn later_hints_win_and_others_are_ignored() {
        let e = effects_of(&[
            "byte:urgency:2",
            "byte:urgency:1",
            "string:category:device",
            "boolean:resident:true",
        ]);
        assert_eq!(e.urgency, Some(Urgency::Normal));
        assert_eq!(e.transient, None);
        // A hint of the wrong type changes nothing.
        assert_eq!(effects_of(&["string:urgency:2"]).urgency, None);
    }

    #[test]
    fn sounds() {
        assert_eq!(
            windows_sound("message-new-instant").as_deref(),
            Some("ms-winsoundevent:Notification.IM")
        );
        assert_eq!(
            windows_sound("mail").as_deref(),
            Some("ms-winsoundevent:Notification.Mail")
        );
        assert_eq!(
            windows_sound("ms-winsoundevent:Notification.Looping.Alarm").as_deref(),
            Some("ms-winsoundevent:Notification.Looping.Alarm")
        );
        assert_eq!(windows_sound("bell"), None);
    }
}
