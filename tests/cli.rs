//! End-to-end tests against the built `notify-send.exe`.
//!
//! Tests that show notifications do so as the app "notify-send tests", with
//! low urgency where they can, which sends a notification straight to the
//! notification center instead of popping it up. They check what Windows
//! received through the notification history, and remove what they showed.
//!
//! Whether Windows can show notifications at all (it cannot in a service, for
//! example) is checked once, without notify-send, so that a broken notify-send
//! fails the tests instead of skipping them. Where it cannot, the tests that
//! need notifications are skipped with a warning that `cargo test` shows. What
//! each option turns into is unit tested in `src/notification.rs` and
//! `src/toast.rs`.

#![cfg(windows)]

// Shared with notify-send itself: the app's AUMID and the notifications' group.
#[path = "../src/identity.rs"]
mod identity;
#[allow(dead_code)]
#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::OnceLock;
use std::thread::{self, JoinHandle, sleep};
use std::time::{Duration, Instant};

use temp_dir::TempDir;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{Error, HSTRING};

/// The app name the tests show their notifications as.
const APP: &str = "notify-send tests";

/// How long a notify-send run may take before the test fails.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Starts the process in a group of its own, so that Ctrl+Break can be sent to it alone.
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

fn exe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_notify-send"))
}

/// A child process that is killed if the test ends before it exits.
struct Running(Child);

impl Running {
    fn new(command: &mut Command) -> Self {
        Self(command.spawn().unwrap())
    }

    fn wait_timeout(&mut self, timeout: Duration) -> Option<ExitStatus> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= end {
                return None;
            }
            sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Runs a command to completion and returns its output. Kills it and fails the
/// test if it takes longer than `TIMEOUT`.
fn run_command(command: &mut Command) -> Output {
    let mut child = Running::new(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    let stdout = read_in_background(child.0.stdout.take().unwrap());
    let stderr = read_in_background(child.0.stderr.take().unwrap());
    let status = child
        .wait_timeout(TIMEOUT)
        .expect("notify-send did not exit");
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}

fn read_in_background(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut out = Vec::new();
        let _ = pipe.read_to_end(&mut out);
        out
    })
}

fn run(args: &[&str]) -> Output {
    run_command(exe().args(args))
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Polls until `f` holds, for up to five seconds.
fn eventually(f: impl Fn() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if f() {
            return true;
        }
        sleep(Duration::from_millis(50));
    }
    f()
}

/// Prints a warning that `cargo test` shows also for passing tests: it
/// captures what tests print with `eprintln!`, but not what they write to
/// stderr directly.
fn warn(message: &str) {
    let _ = writeln!(std::io::stderr(), "{message}");
}

/// The test app's notifications that Windows holds, on screen or in the
/// notification center, as (tag, XML).
fn history() -> Vec<(String, String)> {
    let aumid = HSTRING::from(identity::aumid(APP));
    let Ok(list) = ToastNotificationManager::History().and_then(|h| h.GetHistoryWithId(&aumid))
    else {
        return Vec::new();
    };
    list.into_iter()
        .filter_map(|toast| {
            let tag = toast.Tag().ok()?.to_string();
            let xml = toast.Content().ok()?.GetXml().ok()?.to_string();
            Some((tag, xml))
        })
        .collect()
}

/// The XML of the notification with this ID, if Windows holds it.
fn find(id: &str) -> Option<String> {
    history()
        .into_iter()
        .find(|(tag, _)| tag == id)
        .map(|(_, xml)| xml)
}

/// Takes down the notification with this ID when dropped, so that a failed
/// test leaves nothing behind.
struct Shown(String);

impl Drop for Shown {
    fn drop(&mut self) {
        if let Ok(history) = ToastNotificationManager::History() {
            let _ = history.RemoveGroupedTagWithId(
                &HSTRING::from(&self.0),
                &HSTRING::from(identity::GROUP),
                &HSTRING::from(identity::aumid(APP)),
            );
        }
    }
}

/// Shows a notification as the test app with `args`, which must include `-p`,
/// and returns its ID.
fn show(args: &[&str]) -> Shown {
    let out = run_command(exe().args(["-a", APP]).args(args));
    assert!(out.status.success(), "{}", stderr(&out));
    let id = stdout(&out);
    assert!(
        id.parse::<i32>().is_ok_and(|id| id > 0),
        "expected an ID, got {id:?}"
    );
    Shown(id)
}

/// The test app's registry key, under HKCU.
fn app_key() -> String {
    format!(r"Software\Classes\AppUserModelId\{}", identity::aumid(APP))
}

/// A string value of the test app's registry key.
fn registry_value(name: &str) -> Option<String> {
    let (key, name) = (HSTRING::from(app_key()), HSTRING::from(name));
    let mut buffer = vec![0u16; 4096];
    let mut bytes = u32::try_from(buffer.len() * 2).unwrap();
    // SAFETY: `buffer` holds `bytes` bytes, which RegGetValueW does not exceed.
    let result = unsafe {
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
    result.ok().ok()?;
    let chars = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..chars]))
}

/// Whether Windows can show notifications here; if not, the tests that need
/// it are skipped. Checked by showing one directly, not through notify-send.
fn can_notify() -> bool {
    static CAN_NOTIFY: OnceLock<bool> = OnceLock::new();
    let can = *CAN_NOTIFY.get_or_init(|| match probe() {
        Ok(()) => true,
        Err(e) => {
            warn(&format!(
                "warning: Windows does not show notifications here ({e}), so the tests that need them are skipped"
            ));
            false
        }
    });
    if !can {
        warn("skipped: Windows does not show notifications here");
    }
    can
}

/// Registers the test app as notify-send does, shows a notification straight
/// to the notification center, and checks that Windows holds it.
fn probe() -> windows::core::Result<()> {
    let display_name: Vec<u16> = APP.encode_utf16().chain(Some(0)).collect();
    // SAFETY: a NUL-terminated UTF-16 string and its size in bytes.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(app_key()),
            &HSTRING::from("DisplayName"),
            REG_SZ.0,
            Some(display_name.as_ptr().cast()),
            u32::try_from(display_name.len() * 2).unwrap(),
        )
    }
    .ok()?;
    let content = XmlDocument::new()?;
    content.LoadXml(&HSTRING::from(
        r#"<toast><visual><binding template="ToastGeneric"><text>notify-send tests</text></binding></visual></toast>"#,
    ))?;
    let toast = ToastNotification::CreateToastNotification(&content)?;
    toast.SetTag(&HSTRING::from("probe"))?;
    toast.SetGroup(&HSTRING::from(identity::GROUP))?;
    toast.SetSuppressPopup(true)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(identity::aumid(APP)))?
        .Show(&toast)?;
    let shown = Shown("probe".into());
    if eventually(|| find(&shown.0).is_some()) {
        Ok(())
    } else {
        Err(Error::new(
            E_FAIL,
            "a notification did not reach the history",
        ))
    }
}

#[test]
fn shows_a_notification() {
    if !can_notify() {
        return;
    }
    let shown = show(&["-u", "low", "-p", "Build finished", "All tests passed ✓"]);
    let xml = find(&shown.0).expect("the notification is not in the history");
    assert!(
        xml.contains("<text>Build finished</text><text>All tests passed ✓</text>"),
        "{xml}"
    );
}

#[test]
fn registers_the_app_name_and_icon() {
    if !can_notify() {
        return;
    }
    let _shown = show(&["-u", "low", "-p", "Registration"]);
    assert_eq!(registry_value("DisplayName").as_deref(), Some(APP));
    let icon = registry_value("IconUri").expect("no IconUri");
    let png = fs::read(&icon).unwrap();
    assert!(png.starts_with(b"\x89PNG"), "{icon} is not a PNG");
}

#[test]
fn replace_id_replaces_the_notification() {
    if !can_notify() {
        return;
    }
    let first = show(&["-u", "low", "-p", "Progress", "first"]);
    let second = show(&["-u", "low", "-p", "-r", &first.0, "Progress", "second"]);
    assert_eq!(second.0, first.0, "-p prints the replaced ID");
    let held: Vec<_> = history()
        .into_iter()
        .filter(|(tag, _)| *tag == first.0)
        .collect();
    assert_eq!(held.len(), 1, "{held:?}");
    assert!(held[0].1.contains("<text>second</text>"), "{}", held[0].1);
}

#[test]
fn expire_time_removes_the_notification() {
    if !can_notify() {
        return;
    }
    let shown = show(&["-u", "low", "-t", "1000", "-p", "Expiring"]);
    assert!(eventually(|| find(&shown.0).is_some()), "not shown");
    // Windows removes expired notifications with a delay of up to a few seconds.
    let end = Instant::now() + Duration::from_secs(10);
    while find(&shown.0).is_some() && Instant::now() < end {
        sleep(Duration::from_millis(100));
    }
    assert!(find(&shown.0).is_none(), "not removed when it expired");
}

#[test]
fn the_body_understands_escapes() {
    if !can_notify() {
        return;
    }
    let shown = show(&["-u", "low", "-p", r"C:\new", r"one\ntwo, C:\\new"]);
    // Windows stores new lines in the notification as CRLF.
    let xml = find(&shown.0).unwrap().replace("\r\n", "\n");
    assert!(xml.contains(r"<text>C:\new</text>"), "{xml:?}");
    assert!(xml.contains("<text>one\ntwo, C:\\new</text>"), "{xml:?}");
}

#[test]
fn icons_are_found_relative_to_the_current_directory() {
    if !can_notify() {
        return;
    }
    let dir = TempDir::new("icons");
    fs::copy("assets/notify-send.png", dir.path().join("build ok.png")).unwrap();
    let out = run_command(
        exe()
            .args(["-a", APP, "-u", "low", "-p", "-i", "build ok.png", "Icon"])
            .current_dir(dir.path()),
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let shown = Shown(stdout(&out));
    let xml = find(&shown.0).unwrap();
    let dir_name = dir.path().file_name().unwrap().to_str().unwrap();
    assert!(xml.contains("src=\"file:///"), "{xml}");
    assert!(
        xml.contains(&format!("/{dir_name}/build%20ok.png\"")),
        "{xml}"
    );
}

#[test]
fn missing_icons_are_a_warning() {
    if !can_notify() {
        return;
    }
    let out = run(&["-a", APP, "-u", "low", "-p", "-i", "missing.png", "No icon"]);
    let _shown = Shown(stdout(&out));
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("notify-send: warning: icon not found: missing.png"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn images_on_other_computers_are_refused() {
    if !can_notify() {
        return;
    }
    // The .invalid domain never resolves, should the check ever be skipped.
    let remote = r"\\server.invalid\share\x.png";
    let start = Instant::now();
    let out = run(&[
        "-a", APP, "-u", "low", "-p", "-i", remote, "-n", remote, "Remote",
    ]);
    let shown = Shown(stdout(&out));
    assert!(out.status.success(), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains(&format!(
            "warning: icon not used, since it is not a file on this computer: {remote}"
        )),
        "{err}"
    );
    assert!(err.contains("warning: app icon not used"), "{err}");
    assert!(!find(&shown.0).unwrap().contains("<image"));
    // Looking the path up would have taken a network timeout.
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn wait_ends_at_the_expire_time_and_takes_the_notification_down() {
    if !can_notify() {
        return;
    }
    // A question, which stays on screen until it is answered. On GitHub's
    // runners, Windows reports a notification without buttons as gone at
    // once, which ends the wait.
    let start = Instant::now();
    let out = run(&[
        "-a",
        APP,
        "-p",
        "-A",
        "yes=Yes",
        "-t",
        "1500",
        "-h",
        "boolean:suppress-sound:true",
        "notify-send tests",
        "Waiting for 1.5 seconds",
    ]);
    let elapsed = start.elapsed();
    let shown = Shown(stdout(&out));
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        elapsed >= Duration::from_millis(1400),
        "the wait ended after {elapsed:?}, with {:?}",
        stderr(&out)
    );
    assert!(elapsed < Duration::from_secs(6), "{elapsed:?}");
    // libnotify's message, without a prefix.
    assert_eq!(stderr(&out), "Wait timeout expired\n");
    // It expires two seconds after the wait, so only the wait can have
    // taken it down by now.
    assert!(find(&shown.0).is_none(), "still shown after the wait");
}

#[test]
fn ctrl_break_cancels_a_wait_and_takes_the_notification_down() {
    if !can_notify() {
        return;
    }
    let mut child = Running::new(
        exe()
            .args([
                "-a",
                APP,
                "-p",
                "-A",
                "yes=Yes",
                "-A",
                "no=No",
                "-h",
                "boolean:suppress-sound:true",
                "notify-send tests",
                "Waiting for an answer",
            ])
            .creation_flags(CREATE_NEW_PROCESS_GROUP)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    // The ID is printed once the notification shows, and the wait begins.
    let mut id = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut id)
        .unwrap();
    let shown = Shown(id.trim().to_string());
    let stderr_pipe = read_in_background(child.0.stderr.take().unwrap());
    assert!(eventually(|| find(&shown.0).is_some()), "not shown");
    assert!(
        child.wait_timeout(Duration::from_millis(500)).is_none(),
        "did not wait"
    );
    // SAFETY: targets the process group created for this child alone.
    if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.0.id()) }.is_err() {
        warn("skipped: no console to deliver Ctrl+Break to");
        return;
    }
    let status = child
        .wait_timeout(Duration::from_secs(5))
        .expect("notify-send ignored Ctrl+Break");
    let err = String::from_utf8_lossy(&stderr_pipe.join().unwrap()).into_owned();
    // As in libnotify: a message, and exit status 0.
    assert_eq!(status.code(), Some(0), "{err}");
    assert_eq!(err, "Wait cancelled, closing notification\n");
    assert!(find(&shown.0).is_none(), "still shown after Ctrl+Break");
}

#[test]
fn usage_errors_exit_1_with_libnotifys_messages() {
    let cases: [(&[&str], &str); 7] = [
        (&[], "No summary specified."),
        (&["a", "b", "c"], "Invalid number of options."),
        (
            &["-u", "urgent", "T"],
            "Unknown urgency urgent specified. Known urgency levels: low, normal, critical.",
        ),
        (
            &["-h", "string:x", "T"],
            "Invalid hint syntax specified. Use TYPE:NAME:VALUE.",
        ),
        (
            &["-t", "soon", "T"],
            "Cannot parse integer value “soon” for -t",
        ),
        (&["--nope", "T"], "Unknown option --nope"),
        (&["-at", "App", "1", "T"], "Error parsing option -t"),
    ];
    for (args, message) in cases {
        let out = run(args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert_eq!(stderr(&out), format!("{message}\n"), "{args:?}");
        assert!(stdout(&out).is_empty(), "{args:?}");
    }
}

#[test]
fn help_and_version() {
    for flag in ["-?", "--help", "--help-all"] {
        let help = run(&[flag]);
        assert!(help.status.success());
        assert!(stdout(&help).starts_with("Usage:\n  notify-send [OPTION…] <SUMMARY> [BODY]"));
    }
    let version = run(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        stdout(&version),
        concat!("notify-send ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn does_not_need_the_visual_cpp_runtime() {
    // .cargo/config.toml links the C runtime statically; RUSTFLAGS overrides that.
    let exe = fs::read(env!("CARGO_BIN_EXE_notify-send"))
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        !exe.windows(b"vcruntime".len()).any(|w| w == b"vcruntime"),
        "notify-send.exe needs vcruntime140.dll; is RUSTFLAGS set?"
    );
}
