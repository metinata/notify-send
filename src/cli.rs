//! Argument parsing, mirroring libnotify's `notify-send(1)`, which parses its
//! options with GLib (`glib/goption.c`):
//!
//! - Options can come before or after the summary and body, and `--` ends them.
//! - A long option takes its value as `--name=value` or as the next argument.
//! - A short option that takes a value takes the next argument, also inside a
//!   group: `-ap Backup` is `-a Backup -p`. A second one in the same group is
//!   an error, and a value is never attached: `-ucritical` is an error too.
//! - A flag given a value, as in `--wait=yes`, is set; the value is ignored.
//! - Errors are GLib's and libnotify's messages, word for word.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use crate::escape;
use crate::hints::{self, Hint};

pub const USAGE: &str = "\
Usage:
  notify-send [OPTION…] <SUMMARY> [BODY] - create a notification

Help Options:
  -?, --help                     Show help options

Application Options:
  -u, --urgency=LEVEL            Urgency: low (straight to the notification
                                 center), normal, or critical (stays on screen)
  -t, --expire-time=TIME         Remove the notification after TIME milliseconds;
                                 0 keeps it on screen until it is closed
  -a, --app-name=APP_NAME        Name of the app the notification is from
  -i, --icon=ICON                Image file to show in the notification
  -n, --app-icon=ICON            Image file to use as the app's icon
  -c, --category=TYPE[,TYPE...]  Notification category (accepted, not used)
  -e, --transient                Do not keep the notification in the
                                 notification center
  -h, --hint=TYPE:NAME:VALUE     Extra data; TYPE is boolean, int, double,
                                 string, byte or variant
  -p, --print-id                 Print the notification ID
  -r, --replace-id=REPLACE_ID    Replace the notification with this ID
  -w, --wait                     Wait for the notification to be closed
  -A, --action=[NAME=]Text...    Add a button labeled Text, and wait: prints the
                                 NAME of the button chosen (or its position,
                                 from 0). The NAME default is a click on the
                                 notification itself
  -v, --version                  Print the version

As on Linux, the body understands backslash escapes: \\n starts a new line,
and \\\\ is a backslash. Put -- before text from elsewhere, so that text that
starts with - is not read as an option.
";

/// The characters C's `isspace` matches, which GLib and libnotify trim.
pub const C_WHITESPACE: [char; 6] = [' ', '\t', '\n', '\x0b', '\x0c', '\r'];

/// `-u`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

/// An `-A` action: a button, or with the name `default`, a click on the
/// notification itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub name: String,
    pub label: String,
}

/// `-t` when not given: the notification server decides
/// (`NOTIFY_EXPIRES_DEFAULT` in libnotify).
pub const EXPIRES_DEFAULT: i32 = -1;
/// `-t 0`: the notification never expires (`NOTIFY_EXPIRES_NEVER`).
pub const EXPIRES_NEVER: i32 = 0;

#[derive(Debug, PartialEq)]
pub struct Options {
    pub summary: String,
    /// With its backslash escapes replaced.
    pub body: Option<String>,
    pub urgency: Urgency,
    /// `-t`, in milliseconds; see `EXPIRES_DEFAULT` and `EXPIRES_NEVER`.
    pub expire_time: i32,
    pub app_name: Option<String>,
    pub icon: Option<OsString>,
    pub app_icon: Option<OsString>,
    pub transient: bool,
    pub hints: Vec<Hint>,
    pub print_id: bool,
    /// `-r`; 0 for a new notification.
    pub replace_id: i32,
    /// `-w`, or implied by actions.
    pub wait: bool,
    pub actions: Vec<Action>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            summary: String::new(),
            body: None,
            urgency: Urgency::default(),
            // Not 0, which means never.
            expire_time: EXPIRES_DEFAULT,
            app_name: None,
            icon: None,
            app_icon: None,
            transient: false,
            hints: Vec::new(),
            print_id: false,
            replace_id: 0,
            wait: false,
            actions: Vec::new(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Parsed {
    Show(Options),
    Help,
    Version,
}

/// libnotify's options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opt {
    Urgency,
    ExpireTime,
    AppName,
    Icon,
    AppIcon,
    Category,
    Transient,
    Hint,
    PrintId,
    ReplaceId,
    Wait,
    Action,
    Version,
}

impl Opt {
    const ALL: [(Opt, char, &'static str); 13] = [
        (Opt::Urgency, 'u', "urgency"),
        (Opt::ExpireTime, 't', "expire-time"),
        (Opt::AppName, 'a', "app-name"),
        (Opt::Icon, 'i', "icon"),
        (Opt::AppIcon, 'n', "app-icon"),
        (Opt::Category, 'c', "category"),
        (Opt::Transient, 'e', "transient"),
        (Opt::Hint, 'h', "hint"),
        (Opt::PrintId, 'p', "print-id"),
        (Opt::ReplaceId, 'r', "replace-id"),
        (Opt::Wait, 'w', "wait"),
        (Opt::Action, 'A', "action"),
        (Opt::Version, 'v', "version"),
    ];

    fn by_short(short: char) -> Option<Self> {
        Self::ALL.iter().find(|o| o.1 == short).map(|o| o.0)
    }

    fn by_long(long: &str) -> Option<Self> {
        Self::ALL.iter().find(|o| o.2 == long).map(|o| o.0)
    }

    fn takes_value(self) -> bool {
        !matches!(
            self,
            Opt::Transient | Opt::PrintId | Opt::Wait | Opt::Version
        )
    }
}

/// The options as given, before the summary, body, hints and actions are
/// checked.
#[derive(Default)]
struct Given {
    options: Options,
    texts: Vec<OsString>,
    hints: Vec<String>,
    actions: Vec<String>,
    version: bool,
}

impl Given {
    /// Applies an option. `value` is `Some` for the options that take one;
    /// `option` is its name as given, for messages.
    fn set(&mut self, opt: Opt, value: Option<OsString>, option: &str) -> Result<(), String> {
        let text = |value: Option<OsString>| {
            value.unwrap_or_default().into_string().map_err(|value| {
                format!(
                    "Invalid UTF-8 provided for {option}: {}",
                    value.to_string_lossy()
                )
            })
        };
        let o = &mut self.options;
        match opt {
            Opt::Urgency => o.urgency = parse_urgency(&text(value)?)?,
            Opt::ExpireTime => o.expire_time = parse_int(&text(value)?, option)?,
            Opt::AppName => o.app_name = Some(text(value)?),
            Opt::Icon => o.icon = value,
            Opt::AppIcon => o.app_icon = value,
            // Accepted for compatibility; Windows has no notification categories.
            Opt::Category => {}
            Opt::Transient => o.transient = true,
            Opt::Hint => self.hints.push(text(value)?),
            Opt::PrintId => o.print_id = true,
            Opt::ReplaceId => o.replace_id = parse_int(&text(value)?, option)?,
            Opt::Wait => o.wait = true,
            Opt::Action => self.actions.push(text(value)?),
            Opt::Version => self.version = true,
        }
        Ok(())
    }
}

/// Parses arguments; the first item is the program name, as in
/// `std::env::args_os`. Errors are messages for the user.
pub fn parse<I>(args: I) -> Result<Parsed, String>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().skip(1).map(Into::into).collect();
    let mut given = Given::default();
    let mut options_ended = false;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        let units: Vec<u16> = arg.encode_wide().collect();
        let unknown = || format!("Unknown option {}", arg.to_string_lossy());
        if options_ended || units.len() < 2 || units[0] != u16::from(b'-') {
            given.texts.push(arg.clone());
        } else if units[1] == u16::from(b'-') {
            let rest = &units[2..];
            if rest.is_empty() {
                options_ended = true;
                i += 1;
                continue;
            }
            let (name, attached) = match rest.iter().position(|&u| u == u16::from(b'=')) {
                Some(eq) => (&rest[..eq], Some(OsString::from_wide(&rest[eq + 1..]))),
                None => (rest, None),
            };
            let name = String::from_utf16_lossy(name);
            if attached.is_none() && (name == "help" || name == "help-all") {
                return Ok(Parsed::Help);
            }
            let opt = Opt::by_long(&name).ok_or_else(unknown)?;
            let option = format!("--{name}");
            let value = match (opt.takes_value(), attached) {
                (false, _) => None,
                (true, Some(value)) => Some(value),
                (true, None) => {
                    i += 1;
                    let value = args
                        .get(i)
                        .ok_or(format!("Missing argument for {option}"))?;
                    Some(value.clone())
                }
            };
            given.set(opt, value, &option)?;
        } else {
            // A group of short options, such as -pe.
            let mut value_taken = false;
            for unit in char::decode_utf16(units[1..].iter().copied()) {
                let short = unit.unwrap_or(char::REPLACEMENT_CHARACTER);
                if short == '?' {
                    return Ok(Parsed::Help);
                }
                let opt = Opt::by_short(short).ok_or_else(unknown)?;
                let option = format!("-{short}");
                let value = if !opt.takes_value() {
                    None
                } else if value_taken {
                    return Err(format!("Error parsing option {option}"));
                } else {
                    value_taken = true;
                    let value = args
                        .get(i + 1)
                        .ok_or(format!("Missing argument for {option}"))?;
                    Some(value.clone())
                };
                given.set(opt, value, &option)?;
            }
            if value_taken {
                i += 1;
            }
        }
        i += 1;
    }
    if given.version {
        return Ok(Parsed::Version);
    }
    finish(given).map(Parsed::Show)
}

/// Checks the summary and body, then the hints and actions, in libnotify's order.
fn finish(given: Given) -> Result<Options, String> {
    let mut options = given.options;
    let mut texts = given.texts.into_iter();
    options.summary = texts
        .next()
        .ok_or("No summary specified.")?
        .into_string()
        .map_err(|_| "Invalid UTF-8 provided for parameter: Summary")?;
    if let Some(body) = texts.next() {
        let body = body
            .into_string()
            .ok()
            .and_then(|body| escape::compress(&body))
            .ok_or("Invalid UTF-8 provided for parameter: Body")?;
        options.body = Some(body);
        if texts.next().is_some() {
            return Err("Invalid number of options.".into());
        }
    }
    options.hints = given
        .hints
        .iter()
        .map(|hint| hints::parse(hint))
        .collect::<Result<_, _>>()?;
    options.actions = parse_actions(&given.actions);
    // As in libnotify, only actions that were added imply --wait.
    options.wait |= !options.actions.is_empty();
    Ok(options)
}

fn parse_urgency(level: &str) -> Result<Urgency, String> {
    match level.to_ascii_lowercase().as_str() {
        "low" => Ok(Urgency::Low),
        "normal" => Ok(Urgency::Normal),
        "critical" => Ok(Urgency::Critical),
        _ => Err(format!(
            "Unknown urgency {level} specified. Known urgency levels: low, normal, critical."
        )),
    }
}

/// Parses an integer the way GLib's option parser does, with C's `strtol` in
/// base 0: leading whitespace and a sign are allowed, `0x` starts a hex
/// number and `0` an octal one, and the whole value must be a number.
fn parse_int(value: &str, option: &str) -> Result<i32, String> {
    let invalid = || format!("Cannot parse integer value “{value}” for {option}");
    let out_of_range = || format!("Integer value “{value}” for {option} out of range");
    let text = value.trim_start_matches(C_WHITESPACE);
    let (negative, text) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let (digits, radix) =
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            (hex, 16)
        } else if text.len() > 1 && text.starts_with('0') {
            (&text[1..], 8)
        } else {
            (text, 10)
        };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return Err(invalid());
    }
    let magnitude = i64::from_str_radix(digits, radix).map_err(|_| out_of_range())?;
    let number = if negative { -magnitude } else { magnitude };
    i32::try_from(number).map_err(|_| out_of_range())
}

/// `-A [NAME=]Text`, as libnotify 0.8 reads it: the name and the label are
/// trimmed, an action without a name is named after its position (from 0,
/// counting every `-A`), and one with an empty name or label is left out.
fn parse_actions(raw: &[String]) -> Vec<Action> {
    let trim = |s: &str| s.trim_matches(C_WHITESPACE).to_owned();
    raw.iter()
        .enumerate()
        .filter_map(|(i, action)| {
            let (name, label) = match action.split_once('=') {
                Some((name, label)) => (trim(name), trim(label)),
                None => (i.to_string(), trim(action)),
            };
            (!name.is_empty() && !label.is_empty()).then_some(Action { name, label })
        })
        .collect()
}

/// Parses `args`, given without the program name, into options; for tests.
#[cfg(test)]
pub fn options(args: &[&str]) -> Result<Options, String> {
    match parse(std::iter::once("notify-send").chain(args.iter().copied()))? {
        Parsed::Show(options) => Ok(options),
        other => panic!("expected options, got {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hints::Value;

    fn parse_args(args: &[&str]) -> Result<Parsed, String> {
        parse(std::iter::once("notify-send").chain(args.iter().copied()))
    }

    fn show(args: &[&str]) -> Options {
        options(args).unwrap()
    }

    fn error(args: &[&str]) -> String {
        parse_args(args).unwrap_err()
    }

    #[test]
    fn summary_and_body() {
        let o = show(&["Build finished", "All tests passed"]);
        assert_eq!(o.summary, "Build finished");
        assert_eq!(o.body.as_deref(), Some("All tests passed"));
        assert_eq!(o.urgency, Urgency::Normal);
        assert_eq!(o.expire_time, EXPIRES_DEFAULT);
        assert!(!o.wait && !o.print_id && !o.transient);
        assert_eq!(show(&["Only a summary"]).body, None);
        // An empty summary is allowed, as in libnotify 0.8.
        assert_eq!(show(&[""]).summary, "");
        // A lone dash is text.
        assert_eq!(show(&["-"]).summary, "-");
    }

    #[test]
    fn the_body_has_escapes_and_the_summary_does_not() {
        let o = show(&[r"a\nb", r"c\nd"]);
        assert_eq!(o.summary, r"a\nb");
        assert_eq!(o.body.as_deref(), Some("c\nd"));
    }

    #[test]
    fn options_can_follow_the_text() {
        let o = show(&["Title", "Body", "-u", "critical", "--expire-time=5000"]);
        assert_eq!(o.summary, "Title");
        assert_eq!(o.urgency, Urgency::Critical);
        assert_eq!(o.expire_time, 5000);
    }

    #[test]
    fn double_dash_ends_the_options() {
        let o = show(&["-p", "--", "-5 °C outside", "--not-an-option"]);
        assert!(o.print_id);
        assert_eq!(o.summary, "-5 °C outside");
        assert_eq!(o.body.as_deref(), Some("--not-an-option"));
    }

    #[test]
    fn every_option() {
        let o = show(&[
            "-u",
            "LOW",
            "-t",
            "1500",
            "-a",
            "Backup",
            "-i",
            "icon.png",
            "-n",
            "app.png",
            "-c",
            "transfer.complete",
            "-e",
            "-h",
            "int:value:30",
            "-p",
            "-r",
            "17",
            "-w",
            "-A",
            "ok=OK",
            "Done",
        ]);
        assert_eq!(o.urgency, Urgency::Low);
        assert_eq!(o.expire_time, 1500);
        assert_eq!(o.app_name.as_deref(), Some("Backup"));
        assert_eq!(o.icon, Some("icon.png".into()));
        assert_eq!(o.app_icon, Some("app.png".into()));
        assert!(o.transient && o.print_id && o.wait);
        assert_eq!(
            o.hints,
            [Hint {
                name: "value".into(),
                value: Value::Int(30)
            }]
        );
        assert_eq!(o.replace_id, 17);
        assert_eq!(
            o.actions,
            [Action {
                name: "ok".into(),
                label: "OK".into()
            }]
        );
    }

    #[test]
    fn long_forms() {
        let o = show(&[
            "--app-name",
            "CI",
            "--icon=a.png",
            "--urgency=critical",
            "T",
        ]);
        assert_eq!(o.app_name.as_deref(), Some("CI"));
        assert_eq!(o.icon, Some("a.png".into()));
        assert_eq!(o.urgency, Urgency::Critical);
        // A value may start with a dash.
        assert_eq!(show(&["-t", "-1", "T"]).expire_time, -1);
        assert_eq!(show(&["--expire-time", "-1", "T"]).expire_time, -1);
        // A value may be empty.
        assert_eq!(show(&["--app-name=", "T"]).app_name.as_deref(), Some(""));
    }

    #[test]
    fn groups_of_short_options_take_the_next_argument() {
        let o = show(&["-ap", "MyApp", "Build done"]);
        assert_eq!(o.app_name.as_deref(), Some("MyApp"));
        assert!(o.print_id);
        assert_eq!(o.summary, "Build done");
        let o = show(&["-pea", "Backup", "Done"]);
        assert!(o.print_id && o.transient);
        assert_eq!(o.app_name.as_deref(), Some("Backup"));
        // The value is never attached.
        assert_eq!(
            error(&["-ucritical", "T"]),
            "Unknown urgency T specified. Known urgency levels: low, normal, critical."
        );
        assert_eq!(
            error(&["-ucritical", "low", "T"]),
            "Error parsing option -c"
        );
        // Two options that take a value cannot share one.
        assert_eq!(
            error(&["-at", "App", "100", "T"]),
            "Error parsing option -t"
        );
    }

    #[test]
    fn flags_ignore_a_value() {
        let o = show(&["--wait=yes", "--print-id=1", "--transient=", "T"]);
        assert!(o.wait && o.print_id && o.transient);
    }

    #[test]
    fn integers_follow_glib() {
        let t = |v: &str| show(&["-t", v, "T"]).expire_time;
        assert_eq!(t("0"), 0);
        assert_eq!(t("+250"), 250);
        assert_eq!(t(" 7"), 7);
        assert_eq!(t("0x10"), 16);
        assert_eq!(t("010"), 8);
        assert_eq!(t("-2147483648"), i32::MIN);
        assert_eq!(
            error(&["-t", "5s", "T"]),
            "Cannot parse integer value “5s” for -t"
        );
        assert!(error(&["--expire-time", "", "T"]).starts_with("Cannot parse"));
        assert!(error(&["-t", "09", "T"]).starts_with("Cannot parse"));
        assert!(error(&["-t", "0x", "T"]).starts_with("Cannot parse"));
        assert_eq!(
            error(&["--replace-id=2147483648", "T"]),
            "Integer value “2147483648” for --replace-id out of range"
        );
    }

    #[test]
    fn actions_follow_libnotify() {
        let o = show(&[
            "-A",
            "yes=Yes",
            "-A",
            "Maybe",
            "-A",
            " no = No ",
            "-A",
            "=x",
            "-A",
            "y=",
            "Q",
        ]);
        let names: Vec<_> = o
            .actions
            .iter()
            .map(|a| (a.name.as_str(), a.label.as_str()))
            .collect();
        // Unnamed actions are named after their position, counting from 0.
        assert_eq!(names, [("yes", "Yes"), ("1", "Maybe"), ("no", "No")]);
        assert!(o.wait, "actions imply --wait");
        assert_eq!(show(&["-A", "Yes", "-A", "No", "Q"]).actions[0].name, "0");
        // Only the first = separates the name.
        assert_eq!(show(&["-A", "eq=a=b", "Q"]).actions[0].label, "a=b");
        // Actions that were all left out do not imply --wait.
        assert!(!show(&["-A", "=", "Q"]).wait);
    }

    #[test]
    fn help_and_version() {
        for args in [
            &["-?"][..],
            &["--help"],
            &["--help-all"],
            &["-p?"],
            &["-?x"],
        ] {
            assert_eq!(parse_args(args), Ok(Parsed::Help), "{args:?}");
        }
        assert_eq!(parse_args(&["-v"]), Ok(Parsed::Version));
        // --version needs no summary, but the other arguments must parse.
        assert_eq!(parse_args(&["--version", "-u", "low"]), Ok(Parsed::Version));
        assert!(parse_args(&["--version", "-x"]).is_err());
    }

    #[test]
    fn errors_are_glibs_and_libnotifys() {
        assert_eq!(error(&[]), "No summary specified.");
        assert_eq!(error(&["-u", "low"]), "No summary specified.");
        assert_eq!(error(&["a", "b", "c"]), "Invalid number of options.");
        assert_eq!(
            error(&["-u", "urgent", "T"]),
            "Unknown urgency urgent specified. Known urgency levels: low, normal, critical."
        );
        assert!(error(&["-h", "nonsense", "T"]).starts_with("Invalid hint syntax"));
        assert_eq!(error(&["-x", "T"]), "Unknown option -x");
        assert_eq!(error(&["-px", "T"]), "Unknown option -px");
        assert_eq!(error(&["--nope=1", "T"]), "Unknown option --nope=1");
        assert_eq!(error(&["--help-me", "T"]), "Unknown option --help-me");
        // -h is --hint, not help, so it needs a value.
        assert_eq!(error(&["-h"]), "Missing argument for -h");
        assert_eq!(error(&["T", "--urgency"]), "Missing argument for --urgency");
        assert_eq!(
            error(&["T", r"\777"]),
            "Invalid UTF-8 provided for parameter: Body"
        );
    }

    #[test]
    fn hints_are_checked_after_the_summary() {
        assert_eq!(error(&["-h", "bad"]), "No summary specified.");
    }
}
