//! The toast XML that Windows renders: text, image, progress bar, buttons and
//! sound. See <https://learn.microsoft.com/uwp/schemas/tiles/toastschema/schema-root>.

use std::fmt::Write;
use std::path::Path;

use crate::cli::Action;

/// Windows shows at most five buttons on a notification.
pub const MAX_BUTTONS: usize = 5;

/// What a click on the notification itself reports, when that is an action.
/// Buttons report their position instead, from 0.
pub const CLICK: &str = "default";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound<'a> {
    Default,
    Silent,
    /// An `ms-winsoundevent:` URI.
    Event(&'a str),
}

#[derive(Debug)]
pub struct Toast<'a> {
    pub title: &'a str,
    pub body: Option<&'a str>,
    /// An image file, shown next to the text.
    pub image: Option<&'a Path>,
    /// A percentage, shown as a progress bar.
    pub progress: Option<u8>,
    /// Buttons, at most `MAX_BUTTONS`. Each reports its position rather than
    /// its action's name, so that no name can pass for another, or for a
    /// click on the notification.
    pub buttons: &'a [Action],
    /// Whether a click on the notification itself is an action (`default`),
    /// which reports `CLICK`.
    pub clickable: bool,
    /// Stay on screen until the user acts on it, rather than going to the
    /// notification center after a few seconds.
    pub persistent: bool,
    /// Stay on screen for about 25 seconds instead of the usual few.
    pub long: bool,
    pub sound: Sound<'a>,
}

impl Toast<'_> {
    pub fn xml(&self) -> String {
        let mut xml = String::from("<toast");
        if self.clickable {
            attribute(&mut xml, "launch", CLICK);
        }
        if self.long {
            attribute(&mut xml, "duration", "long");
        }
        if self.persistent {
            // Stays on screen until dismissed, but only with a button, so a
            // persistent toast without buttons gets the system's Dismiss button.
            attribute(&mut xml, "scenario", "reminder");
        }
        xml.push_str(r#"><visual><binding template="ToastGeneric">"#);
        for text in std::iter::once(self.title).chain(self.body) {
            xml.push_str("<text>");
            escape(&mut xml, text);
            xml.push_str("</text>");
        }
        if let Some(image) = self.image {
            xml.push_str(r#"<image placement="appLogoOverride""#);
            attribute(&mut xml, "src", &file_uri(image));
            xml.push_str("/>");
        }
        if let Some(percent) = self.progress {
            // status is required; empty, it shows nothing but the bar and the percentage.
            let _ = write!(
                xml,
                r#"<progress value="{}" status=""/>"#,
                f64::from(percent) / 100.0
            );
        }
        xml.push_str("</binding></visual>");
        if !self.buttons.is_empty() || self.persistent {
            xml.push_str("<actions>");
            for (position, button) in self.buttons.iter().enumerate() {
                xml.push_str("<action");
                attribute(&mut xml, "content", &button.label);
                attribute(&mut xml, "arguments", &position.to_string());
                xml.push_str("/>");
            }
            if self.buttons.is_empty() {
                // Windows labels it ("Dismiss") in the user's language.
                xml.push_str(r#"<action activationType="system" arguments="dismiss" content=""/>"#);
            }
            xml.push_str("</actions>");
        }
        match self.sound {
            Sound::Default => {}
            Sound::Silent => xml.push_str(r#"<audio silent="true"/>"#),
            Sound::Event(src) => {
                xml.push_str("<audio");
                attribute(&mut xml, "src", src);
                xml.push_str("/>");
            }
        }
        xml.push_str("</toast>");
        xml
    }
}

fn attribute(xml: &mut String, name: &str, value: &str) {
    let _ = write!(xml, " {name}=\"");
    escape(xml, value);
    xml.push('"');
}

/// Appends `text` with the XML special characters escaped. Characters that XML
/// does not allow at all (most control characters) are left out.
fn escape(xml: &mut String, text: &str) {
    for c in text.chars().filter(|&c| allowed_in_xml(c)) {
        match c {
            '&' => xml.push_str("&amp;"),
            '<' => xml.push_str("&lt;"),
            '>' => xml.push_str("&gt;"),
            '"' => xml.push_str("&quot;"),
            '\'' => xml.push_str("&apos;"),
            _ => xml.push(c),
        }
    }
}

/// Whether XML 1.0 allows the character; of the control characters, only
/// tab, line feed and carriage return.
fn allowed_in_xml(c: char) -> bool {
    !matches!(c, '\0'..='\x08' | '\x0b' | '\x0c' | '\x0e'..='\x1f' | '\u{fffe}' | '\u{ffff}')
}

/// A `file:` URI for an absolute path: `C:\a b\x.png` becomes
/// `file:///C:/a%20b/x.png`, and `\\server\share\x.png` becomes
/// `file://server/share/x.png`. ASCII characters outside the unreserved set
/// are percent-encoded, so `#` and `%` in a file name stay part of it. Other
/// characters stay as they are (an IRI): Windows does not find images whose
/// URI has them percent-encoded as UTF-8.
pub fn file_uri(path: &Path) -> String {
    let path = path.to_string_lossy();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    let (prefix, rest) = match path
        .strip_prefix(r"UNC\")
        .or_else(|| path.strip_prefix(r"\\"))
    {
        Some(unc) => ("file://", unc),
        None => ("file:///", path),
    };
    let mut uri = String::from(prefix);
    for c in rest.chars() {
        match c {
            '\\' => uri.push('/'),
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' | '/' | ':' => uri.push(c),
            _ if c.is_ascii() => {
                let _ = write!(uri, "%{:02X}", u32::from(c));
            }
            _ => uri.push(c),
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain<'a>(title: &'a str, body: Option<&'a str>) -> Toast<'a> {
        Toast {
            title,
            body,
            image: None,
            progress: None,
            buttons: &[],
            clickable: false,
            persistent: false,
            long: false,
            sound: Sound::Default,
        }
    }

    const START: &str = r#"<toast><visual><binding template="ToastGeneric">"#;
    const END: &str = "</binding></visual></toast>";

    #[test]
    fn title_and_body() {
        assert_eq!(
            plain("Build", Some("Done")).xml(),
            format!("{START}<text>Build</text><text>Done</text>{END}")
        );
        assert_eq!(
            plain("Only", None).xml(),
            format!("{START}<text>Only</text>{END}")
        );
    }

    #[test]
    fn text_is_escaped() {
        let xml = plain(r#"<b>"Tom" & 'Jerry'</b>"#, Some("a\nb\tc\u{7}d")).xml();
        assert!(
            xml.contains("<text>&lt;b&gt;&quot;Tom&quot; &amp; &apos;Jerry&apos;&lt;/b&gt;</text>")
        );
        // New lines and tabs stay; other control characters are not allowed in XML.
        assert!(xml.contains("<text>a\nb\tcd</text>"));
    }

    #[test]
    fn image_and_progress() {
        let toast = Toast {
            image: Some(Path::new(r"C:\Icons\build ok.png")),
            progress: Some(60),
            ..plain("T", None)
        };
        let xml = toast.xml();
        assert!(xml.contains(
            r#"<image placement="appLogoOverride" src="file:///C:/Icons/build%20ok.png"/>"#
        ));
        assert!(xml.contains(r#"<progress value="0.6" status=""/>"#));
        let full = Toast {
            progress: Some(100),
            ..plain("T", None)
        }
        .xml();
        assert!(full.contains(r#"value="1""#));
    }

    #[test]
    fn buttons_and_default_action() {
        let buttons = [
            Action {
                name: "yes".into(),
                label: "Yes".into(),
            },
            Action {
                name: "x&y".into(),
                label: "<No>".into(),
            },
        ];
        let toast = Toast {
            buttons: &buttons,
            clickable: true,
            persistent: true,
            ..plain("Deploy?", None)
        };
        // Buttons report their position, not their action's name.
        assert_eq!(
            toast.xml(),
            format!(
                "{}{START_BODY}<text>Deploy?</text></binding></visual><actions>\
                 <action content=\"Yes\" arguments=\"0\"/>\
                 <action content=\"&lt;No&gt;\" arguments=\"1\"/></actions></toast>",
                r#"<toast launch="default" scenario="reminder">"#,
                START_BODY = r#"<visual><binding template="ToastGeneric">"#,
            )
        );
    }

    #[test]
    fn persistent_toasts_without_buttons_get_dismiss() {
        let xml = Toast {
            persistent: true,
            ..plain("Battery low", None)
        }
        .xml();
        assert!(xml.starts_with(r#"<toast scenario="reminder">"#));
        assert!(xml.contains(
            r#"<actions><action activationType="system" arguments="dismiss" content=""/></actions>"#
        ));
    }

    #[test]
    fn duration_and_sound() {
        let xml = Toast {
            long: true,
            sound: Sound::Silent,
            ..plain("T", None)
        }
        .xml();
        assert!(xml.starts_with(r#"<toast duration="long">"#));
        assert!(xml.ends_with(r#"<audio silent="true"/></toast>"#));
        let xml = Toast {
            sound: Sound::Event("ms-winsoundevent:Notification.IM"),
            ..plain("T", None)
        }
        .xml();
        assert!(xml.ends_with(r#"<audio src="ms-winsoundevent:Notification.IM"/></toast>"#));
    }

    #[test]
    fn file_uris() {
        let uri = |p: &str| file_uri(Path::new(p));
        assert_eq!(uri(r"C:\Users\me\icon.png"), "file:///C:/Users/me/icon.png");
        assert_eq!(uri(r"C:\a #1\50%.png"), "file:///C:/a%20%231/50%25.png");
        assert_eq!(
            uri(r"C:\İkon klasörü\ğ.png"),
            "file:///C:/İkon%20klasörü/ğ.png"
        );
        assert_eq!(uri(r"\\server\share\i.png"), "file://server/share/i.png");
        assert_eq!(uri(r"\\?\C:\long\i.png"), "file:///C:/long/i.png");
        assert_eq!(
            uri(r"\\?\UNC\server\share\i.png"),
            "file://server/share/i.png"
        );
    }
}
