//! What to show and how, worked out from the options: the toast, the app it
//! comes from, how long it lasts, and whether to wait for it.
//!
//! See `docs/design.md` for why each option maps the way it does.

use std::ffi::OsStr;
use std::hash::{BuildHasher, RandomState};
use std::path::{Component, Path, PathBuf, Prefix};
use std::time::Duration;

use windows::Win32::Storage::FileSystem::GetDriveTypeW;
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETMESSAGEDURATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::HSTRING;

use crate::app::App;
use crate::cli::{Action, EXPIRES_NEVER, Options, Urgency};
use crate::hints;
use crate::identity;
use crate::toast::{self, Sound, Toast};

/// The action name that stands for a click on the notification itself, as
/// in the freedesktop.org notification spec.
pub const DEFAULT_ACTION: &str = toast::CLICK;

/// How long Windows shows a `duration="long"` notification.
const LONG_BANNER: Duration = Duration::from_secs(25);

/// How long a transient notification outlives its banner, which Windows
/// takes a moment longer than the setting to hide.
const TRANSIENT_GRACE: Duration = Duration::from_secs(2);

/// How long a notification that is waited for outlives the wait timeout. The
/// wait takes it down itself when the timeout passes; the expiry is only for
/// when notify-send is ended before that.
const WAIT_GRACE: Duration = Duration::from_secs(2);

/// `GetDriveTypeW`'s value for a network drive.
const DRIVE_REMOTE: u32 = 4;

/// The image file extensions that `-i` and `-n` values are told apart from
/// icon names by; Windows shows the first four.
const IMAGE_EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "svg", "tif", "tiff",
];

/// The environment, passed in so that tests control it.
pub struct Context {
    /// What relative image paths are relative to.
    pub current_dir: PathBuf,
    /// How long notifications stay on screen.
    pub banner_time: Duration,
    /// The ID for a new notification.
    pub new_id: i32,
}

impl Context {
    pub fn current() -> Self {
        Self {
            current_dir: std::env::current_dir().unwrap_or_default(),
            banner_time: banner_time(),
            new_id: new_id(),
        }
    }
}

/// The user's setting for how long notifications stay on screen (Settings →
/// Accessibility → Visual effects → "Dismiss notifications after this amount
/// of time"), 5 seconds by default.
fn banner_time() -> Duration {
    let mut seconds = 0u32;
    // SAFETY: SPI_GETMESSAGEDURATION writes a single u32 to the pointer.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETMESSAGEDURATION,
            0,
            Some((&raw mut seconds).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    match ok {
        Ok(()) if seconds > 0 => Duration::from_secs(seconds.into()),
        _ => Duration::from_secs(5),
    }
}

/// A random ID from 1 to `i32::MAX`. libnotify's IDs come from the
/// notification server, which counts up; here nothing keeps count, and a
/// random ID is as good for `-r`.
fn new_id() -> i32 {
    // RandomState is seeded from the operating system's random number generator.
    let random = RandomState::new().hash_one(std::process::id());
    i32::try_from(random % i32::MAX as u64).expect("below i32::MAX") + 1
}

#[derive(Debug)]
pub struct Notification {
    pub app: App,
    pub id: i32,
    /// Whether the ID is that of an earlier notification (`-r`), which this one replaces.
    pub replace: bool,
    pub xml: String,
    /// The names of the buttons' actions, by position.
    pub buttons: Vec<String>,
    /// Whether a click on the notification is an action (`default`).
    pub clickable: bool,
    /// Pop up on screen; otherwise it goes straight to the notification center.
    pub popup: bool,
    pub high_priority: bool,
    /// Remove the notification after this long.
    pub expires: Option<Duration>,
    /// Remove the notification from the notification center when Windows restarts.
    pub expires_on_reboot: bool,
    pub wait: bool,
    /// Stop waiting after this long.
    pub wait_timeout: Option<Duration>,
    /// Remove the notification when its banner goes away while waiting.
    pub transient: bool,
    pub print_id: bool,
    /// Problems that do not stop the notification, such as a missing icon.
    pub warnings: Vec<String>,
}

pub fn plan(opts: &Options, cx: &Context) -> Result<Notification, String> {
    let effects = hints::effects(&opts.hints);
    let urgency = effects.urgency.unwrap_or(opts.urgency);
    let transient = effects.transient.unwrap_or(opts.transient);
    let mut warnings = Vec::new();

    let image = effects
        .image_path
        .as_deref()
        .map(OsStr::new)
        .or(opts.icon.as_deref())
        .and_then(|icon| find_image(icon, &cx.current_dir, "icon", &mut warnings));
    let app_icon = opts
        .app_icon
        .as_deref()
        .and_then(|icon| find_image(icon, &cx.current_dir, "app icon", &mut warnings));
    let name = opts
        .app_name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or(identity::DEFAULT_NAME);

    let buttons: Vec<Action> = opts
        .actions
        .iter()
        .filter(|action| action.name != DEFAULT_ACTION)
        .cloned()
        .collect();
    if buttons.len() > toast::MAX_BUTTONS {
        return Err(format!(
            "Windows shows at most {} actions as buttons, not {}.",
            toast::MAX_BUTTONS,
            buttons.len()
        ));
    }
    let clickable = buttons.len() < opts.actions.len();

    // libnotify takes an ID from 0 up; on Linux, a negative one is refused
    // with a warning, and the notification shown as a new one.
    let replace_id = if opts.replace_id < 0 {
        warnings.push(format!(
            "the ID {} is out of range, so the notification is shown as a new one",
            opts.replace_id
        ));
        0
    } else {
        opts.replace_id
    };

    let timeout = u64::try_from(opts.expire_time)
        .ok()
        .filter(|&ms| ms > 0)
        .map(Duration::from_millis);
    // Critical notifications stay on screen on Linux, as do ones that never
    // expire. So do ones with actions here, since a script is waiting for the
    // answer (see docs/design.md).
    let persistent = urgency == Urgency::Critical
        || opts.expire_time == EXPIRES_NEVER
        || !opts.actions.is_empty();
    // A long banner stays up for about 25 seconds, which would cut short the
    // banners of someone who set notifications to stay up longer.
    let long = !persistent
        && cx.banner_time < LONG_BANNER
        && timeout.is_some_and(|timeout| timeout > cx.banner_time);
    let sound_event = effects.sound_name.as_deref().and_then(hints::windows_sound);
    let sound = match &sound_event {
        _ if effects.suppress_sound => Sound::Silent,
        Some(event) => Sound::Event(event),
        None => Sound::Default,
    };
    let xml = Toast {
        title: &opts.summary,
        body: opts.body.as_deref(),
        image: image.as_deref(),
        progress: effects.progress,
        buttons: &buttons,
        clickable,
        persistent,
        long,
        sound,
    }
    .xml();

    // A transient notification goes away with its banner, or before, if -t
    // says so. One that is waited for is taken down by the wait when -t
    // passes, and expires a moment later.
    let banner = if long { LONG_BANNER } else { cx.banner_time };
    let banner_end = (transient && !persistent).then(|| banner + TRANSIENT_GRACE);
    let timeout_end = timeout.map(|timeout| {
        if opts.wait {
            timeout + WAIT_GRACE
        } else {
            timeout
        }
    });
    let expires = match (timeout_end, banner_end) {
        (Some(timeout_end), Some(banner_end)) => Some(timeout_end.min(banner_end)),
        (timeout_end, banner_end) => timeout_end.or(banner_end),
    };

    Ok(Notification {
        app: App::new(name, app_icon),
        id: if replace_id == 0 {
            cx.new_id
        } else {
            replace_id
        },
        replace: replace_id != 0,
        xml,
        buttons: buttons.into_iter().map(|button| button.name).collect(),
        clickable,
        // A low-urgency notification never shows on screen, and its wait would
        // end at once, so one with actions shows anyway.
        popup: urgency != Urgency::Low || !opts.actions.is_empty(),
        high_priority: urgency == Urgency::Critical,
        expires,
        // Windows does not tell notify-send when the user signs out or shuts
        // down, so buttons nobody listens to any more go at the next restart.
        expires_on_reboot: transient || !opts.actions.is_empty(),
        wait: opts.wait,
        wait_timeout: timeout,
        transient,
        print_id: opts.print_id,
        warnings,
    })
}

/// The image file that an `-i` or `-n` value names, as an absolute path.
/// `file://` URIs are accepted too. Linux icon names such as
/// `dialog-information` or `org.gnome.Nautilus` have no counterpart on
/// Windows and are ignored; a value that looks like a path but names no file
/// gets a warning.
///
/// Only files on this computer are used. Anything else is refused before it
/// is touched: looking a file up on another computer makes Windows connect to
/// it and sign in with the user's credentials, and the value may come from a
/// commit message or a model's output.
fn find_image(
    value: &OsStr,
    cwd: &Path,
    what: &str,
    warnings: &mut Vec<String>,
) -> Option<PathBuf> {
    let text = value.to_string_lossy();
    let path = match text.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("file://") => path_from_file_uri(&text[7..]),
        _ => PathBuf::from(value),
    };
    // The path as Windows will open it, worked out without touching it:
    // std::path::absolute only rewrites the text, which turns device names
    // such as NUL into \\.\ paths too.
    let Some(absolute) = std::path::absolute(cwd.join(&path))
        .ok()
        .filter(|path| is_local(path))
    else {
        warnings.push(format!(
            "{what} not used, since it is not a file on this computer: {text}"
        ));
        return None;
    };
    if absolute.is_file() {
        return Some(absolute);
    }
    let image_extension = path.extension().is_some_and(|extension| {
        IMAGE_EXTENSIONS
            .iter()
            .any(|image| extension.eq_ignore_ascii_case(image))
    });
    if text.contains(['/', '\\', ':']) || image_extension {
        warnings.push(format!("{what} not found: {text}"));
    }
    None
}

/// Whether an absolute path is on a drive of this computer. Network paths
/// (`\\server\share`, also as `\\?\UNC\server\share` or, for WebDAV,
/// `\\server@SSL\share`), device paths (`\\.\`, `\\?\GLOBALROOT`) and mapped
/// network drives are not.
fn is_local(path: &Path) -> bool {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => !is_network_drive(letter),
            _ => false,
        },
        _ => false,
    }
}

/// Whether a drive letter is mapped to a network share. Windows knows that
/// from the mapping, without contacting the server.
fn is_network_drive(letter: u8) -> bool {
    let root = HSTRING::from(format!(r"{}:\", char::from(letter)));
    // SAFETY: `root` is a NUL-terminated wide string.
    unsafe { GetDriveTypeW(&root) == DRIVE_REMOTE }
}

/// The path in a `file://` URI, after the `file://`.
fn path_from_file_uri(uri: &str) -> PathBuf {
    let path = percent_decode(uri);
    // file://localhost/C:/dir/file.png names a local file, like file:///C:/...
    let path = match path.get(..10) {
        Some(host) if host.eq_ignore_ascii_case("localhost/") => &path[9..],
        _ => &path,
    };
    match path.strip_prefix('/') {
        // file:///C:/dir/file.png
        Some(local) if local.as_bytes().get(1) == Some(&b':') => PathBuf::from(local),
        Some(_) => PathBuf::from(path),
        // file://server/share/file.png
        None => PathBuf::from(format!(r"\\{path}")),
    }
}

/// Decodes `%XX` escapes; a `%` that two hex digits do not follow stays.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        let hex = bytes
            .get(i + 1..i + 3)
            .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))
            .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok());
        match hex {
            Some(decoded) if byte == b'%' => {
                out.push(decoded);
                i += 3;
            }
            _ => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli;
    use crate::temp_dir::TempDir;

    fn context(dir: &Path) -> Context {
        Context {
            current_dir: dir.to_path_buf(),
            banner_time: Duration::from_secs(5),
            new_id: 4242,
        }
    }

    fn plan_in(dir: &Path, args: &[&str]) -> Result<Notification, String> {
        plan(&cli::options(args).unwrap(), &context(dir))
    }

    fn plan_of(args: &[&str]) -> Notification {
        plan_in(&std::env::temp_dir(), args).unwrap()
    }

    #[test]
    fn a_plain_notification() {
        let n = plan_of(&["Build finished", "All tests passed"]);
        assert_eq!(n.app, App::new(identity::DEFAULT_NAME, None));
        assert_eq!(n.id, 4242);
        assert!(n.popup && !n.high_priority && !n.wait && !n.transient && !n.expires_on_reboot);
        assert_eq!((n.expires, n.wait_timeout), (None, None));
        assert!(
            n.xml
                .contains("<text>Build finished</text><text>All tests passed</text>")
        );
        assert!(!n.xml.contains("scenario"));
    }

    #[test]
    fn urgency() {
        let low = plan_of(&["-u", "low", "T"]);
        assert!(
            !low.popup,
            "low urgency goes straight to the notification center"
        );
        assert!(
            plan_of(&["-u", "low", "-A", "Yes", "Q"]).popup,
            "unless a script waits for an answer"
        );
        let critical = plan_of(&["-u", "critical", "T"]);
        assert!(critical.popup && critical.high_priority);
        assert!(critical.xml.contains(r#"scenario="reminder""#));
        assert!(critical.xml.contains(r#"arguments="dismiss""#));
        // The urgency hint overrides -u.
        assert!(!plan_of(&["-u", "critical", "-h", "byte:urgency:0", "T"]).popup);
    }

    #[test]
    fn expire_time() {
        let n = plan_of(&["-t", "3000", "T"]);
        assert_eq!(n.expires, Some(Duration::from_secs(3)));
        assert!(!n.xml.contains("duration"));
        // Longer than the banner: a long banner, which stays up at least as long.
        let n = plan_of(&["-t", "12000", "T"]);
        assert_eq!(n.expires, Some(Duration::from_secs(12)));
        assert!(n.xml.contains(r#"duration="long""#));
        // 0 never expires: it stays on screen.
        let n = plan_of(&["-t", "0", "T"]);
        assert_eq!(n.expires, None);
        assert!(n.xml.contains(r#"scenario="reminder""#));
        // Negative values are the default.
        assert_eq!(plan_of(&["-t", "-1", "T"]).expires, None);
    }

    #[test]
    fn long_banners_do_not_cut_longer_settings_short() {
        let cx = Context {
            banner_time: Duration::from_secs(60),
            ..context(&std::env::temp_dir())
        };
        let n = plan(&cli::options(&["-t", "90000", "T"]).unwrap(), &cx).unwrap();
        assert!(!n.xml.contains("duration"), "{}", n.xml);
        assert_eq!(n.expires, Some(Duration::from_secs(90)));
    }

    #[test]
    fn transient_notifications_go_away_with_their_banner() {
        let n = plan_of(&["-e", "T"]);
        assert!(n.transient && n.expires_on_reboot);
        assert_eq!(n.expires, Some(Duration::from_secs(5) + TRANSIENT_GRACE));
        assert!(plan_of(&["-h", "boolean:transient:true", "T"]).transient);
        // A later hint turns -e off again, as in libnotify.
        assert!(!plan_of(&["-e", "-h", "boolean:transient:false", "T"]).transient);
        // A shorter timeout wins; a longer one does not keep it longer.
        assert_eq!(
            plan_of(&["-e", "-t", "900", "T"]).expires,
            Some(Duration::from_millis(900))
        );
        assert_eq!(
            plan_of(&["-e", "-t", "60000", "T"]).expires,
            Some(LONG_BANNER + TRANSIENT_GRACE)
        );
        // Persistent ones stay until closed.
        assert_eq!(plan_of(&["-e", "-u", "critical", "T"]).expires, None);
    }

    #[test]
    fn actions() {
        let n = plan_of(&["-A", "default=Open", "-A", "yes=Yes", "-A", "No", "Deploy?"]);
        assert!(n.wait && n.clickable && n.expires_on_reboot);
        // Unnamed actions are named after their position among the -A, from 0.
        assert_eq!(n.buttons, ["yes", "2"]);
        assert!(
            n.xml
                .starts_with(r#"<toast launch="default" scenario="reminder">"#)
        );
        assert!(n.xml.contains(
            r#"<action content="Yes" arguments="0"/><action content="No" arguments="1"/>"#
        ));
        assert!(
            !n.xml.contains(r#"content="Open""#),
            "default is not a button"
        );
        assert!(
            !n.xml.contains("dismiss"),
            "the user's buttons keep it on screen"
        );
        assert!(!plan_of(&["-A", "Yes", "Q"]).clickable);

        let labels: Vec<String> = (1..=6).map(|i| format!("Choice {i}")).collect();
        let mut args: Vec<&str> = labels
            .iter()
            .flat_map(|label| ["-A", label.as_str()])
            .collect();
        args.push("T");
        assert!(
            plan_in(&std::env::temp_dir(), &args)
                .unwrap_err()
                .contains("at most 5")
        );
    }

    #[test]
    fn wait_timeout_is_the_expire_time() {
        let n = plan_of(&["-w", "-t", "2000", "T"]);
        assert!(n.wait);
        assert_eq!(n.wait_timeout, Some(Duration::from_secs(2)));
        assert_eq!(plan_of(&["-w", "T"]).wait_timeout, None);
    }

    #[test]
    fn waited_notifications_expire_after_the_wait() {
        let n = plan_of(&["-w", "-t", "2000", "T"]);
        assert_eq!(n.expires, Some(Duration::from_secs(2) + WAIT_GRACE));
        let n = plan_of(&["-A", "Yes", "-t", "1500", "Q"]);
        assert_eq!(n.expires, Some(Duration::from_millis(1500) + WAIT_GRACE));
        // A transient one still goes with its banner, if that is sooner.
        assert_eq!(
            plan_of(&["-e", "-w", "-t", "60000", "T"]).expires,
            Some(LONG_BANNER + TRANSIENT_GRACE)
        );
        assert_eq!(plan_of(&["-w", "T"]).expires, None);
    }

    #[test]
    fn replace_id_and_print_id() {
        let n = plan_of(&["-r", "17", "-p", "T"]);
        assert_eq!(n.id, 17);
        assert!(n.replace && n.print_id);
        let n = plan_of(&["-r", "0", "T"]);
        assert_eq!(n.id, 4242, "-r 0 is a new notification");
        assert!(!n.replace);
        // A negative ID is out of range: a new notification, with a warning.
        let n = plan_of(&["-r", "-5", "T"]);
        assert_eq!(n.id, 4242);
        assert!(!n.replace);
        assert_eq!(n.warnings.len(), 1, "{:?}", n.warnings);
    }

    #[test]
    fn app_name() {
        assert_eq!(
            plan_of(&["-a", "Backup", "T"]).app,
            App::new("Backup", None)
        );
        // An empty name is the default one.
        assert_eq!(plan_of(&["-a", "", "T"]).app.name, identity::DEFAULT_NAME);
    }

    #[test]
    fn images() {
        let dir = TempDir::new("images");
        let icon = dir.file("build ok.png", "png");
        let n = plan_in(
            dir.path(),
            &["-i", "build ok.png", "-n", icon.to_str().unwrap(), "T"],
        )
        .unwrap();
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
        assert!(n.xml.contains(&toast::file_uri(&icon)), "{}", n.xml);
        assert_eq!(n.app.icon.as_deref(), Some(icon.as_path()));

        // file:// URIs, in any case, and the image-path hint, which wins over -i.
        for scheme in ["file://", "FILE://"] {
            let uri = toast::file_uri(&icon).replacen("file://", scheme, 1);
            let hint = format!("string:image-path:{uri}");
            let n = plan_in(dir.path(), &["-i", "missing.png", "-h", &hint, "T"]).unwrap();
            assert!(n.xml.contains("build%20ok.png"), "{uri}: {}", n.xml);
            assert!(n.warnings.is_empty(), "-i is not used: {:?}", n.warnings);
        }
        let localhost = toast::file_uri(&icon).replacen("file://", "file://localhost", 1);
        let n = plan_in(dir.path(), &["-i", &localhost, "T"]).unwrap();
        assert!(n.warnings.is_empty(), "{localhost}: {:?}", n.warnings);
    }

    #[test]
    fn missing_images() {
        let dir = TempDir::new("missing-images");
        // Linux icon names: nothing to show, nothing to warn about.
        for name in ["dialog-information", "org.gnome.Nautilus"] {
            let n = plan_in(dir.path(), &["-i", name, "-n", name, "T"]).unwrap();
            assert!(n.warnings.is_empty(), "{name}: {:?}", n.warnings);
            assert!(!n.xml.contains("<image"));
        }
        // A path that names no file is shown anyway, with a warning.
        let n = plan_in(
            dir.path(),
            &["-i", "missing.png", "-n", r"C:\nope\app.ico", "T"],
        )
        .unwrap();
        assert_eq!(
            n.warnings,
            [
                "icon not found: missing.png",
                r"app icon not found: C:\nope\app.ico"
            ]
        );
        assert_eq!(n.app.icon, None);
    }

    #[test]
    fn images_not_on_this_computer_are_refused() {
        let dir = TempDir::new("remote-images");
        // The .invalid domain never resolves, should the check ever be skipped.
        for value in [
            r"\\server.invalid\share\x.png",
            "//server.invalid/share/x.png",
            "file://server.invalid/share/x.png",
            "file:////server.invalid/share/x.png",
            r"\\server.invalid@SSL@443\share\x.png",
            r"\\?\UNC\server.invalid\share\x.png",
            r"\\.\pipe\notify-send-test",
            // Device names, which Windows turns into \\.\ paths.
            "NUL",
            "file:///C:/x/NUL",
        ] {
            let n = plan_in(dir.path(), &["-i", value, "-n", value, "T"]).unwrap();
            assert!(!n.xml.contains("<image"), "{value}: {}", n.xml);
            assert_eq!(n.app.icon, None, "{value}");
            assert_eq!(
                n.warnings,
                [
                    format!("icon not used, since it is not a file on this computer: {value}"),
                    format!("app icon not used, since it is not a file on this computer: {value}"),
                ]
            );
        }
    }

    #[test]
    fn local_images_in_any_form_are_used() {
        let dir = TempDir::new("local-images");
        let icon = dir.file("icon.png", "png");
        let verbatim = format!(r"\\?\{}", icon.display());
        for value in [icon.to_str().unwrap(), "icon.png", r".\icon.png", &verbatim] {
            let n = plan_in(dir.path(), &["-i", value, "T"]).unwrap();
            assert!(n.warnings.is_empty(), "{value}: {:?}", n.warnings);
            assert!(n.xml.contains("icon.png\"/>"), "{value}: {}", n.xml);
        }
    }

    #[test]
    fn sounds() {
        assert!(
            plan_of(&["-h", "boolean:suppress-sound:true", "T"])
                .xml
                .contains(r#"<audio silent="true"/>"#)
        );
        assert!(
            plan_of(&["-h", "string:sound-name:message-new-email", "T"])
                .xml
                .contains("Notification.Mail")
        );
        assert!(
            !plan_of(&["-h", "string:sound-name:bell", "T"])
                .xml
                .contains("<audio")
        );
    }

    #[test]
    fn progress() {
        assert!(
            plan_of(&["-h", "int:value:42", "T"])
                .xml
                .contains(r#"<progress value="0.42""#)
        );
    }

    #[test]
    fn file_uri_paths() {
        assert_eq!(
            path_from_file_uri("/C:/a%20b/c.png"),
            PathBuf::from("C:/a b/c.png")
        );
        assert_eq!(
            path_from_file_uri("localhost/C:/a/c.png"),
            PathBuf::from("C:/a/c.png")
        );
        assert_eq!(
            path_from_file_uri("server/share/c.png"),
            PathBuf::from(r"\\server/share/c.png")
        );
        assert_eq!(
            path_from_file_uri("/usr/share/icons/x.png"),
            PathBuf::from("/usr/share/icons/x.png")
        );
        assert_eq!(percent_decode("%C4%9F%zz%4"), "ğ%zz%4");
        // Only hex digits make an escape.
        assert_eq!(percent_decode("50%+1.png"), "50%+1.png");
    }

    #[test]
    fn new_ids_are_positive() {
        for _ in 0..100 {
            assert!(new_id() > 0);
        }
    }
}
