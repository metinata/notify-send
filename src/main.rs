//! `notify-send` for Windows: shows desktop notifications, with the same
//! command line as libnotify's `notify-send(1)` on Linux.
//!
//! See `docs/design.md` for how the options map to Windows notifications.

#[cfg(not(windows))]
compile_error!("notify-send-win only runs on Windows. On Linux, use libnotify's notify-send.");

mod app;
mod cli;
mod console;
mod escape;
mod hints;
mod identity;
mod notification;
mod show;
mod toast;

#[cfg(test)]
#[path = "../tests/support/temp_dir.rs"]
mod temp_dir;

use std::io::{self, Write};

use cli::Parsed;
use notification::{Context, Notification};
use show::{Setting, Toast, Waited};

const EXIT_FAILURE: i32 = 1;

fn main() {
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    // Errors are printed as libnotify prints them: the message alone.
    let result = cli::parse(std::env::args_os()).and_then(|parsed| match parsed {
        Parsed::Show(opts) => notification::plan(&opts, &Context::current()).and_then(|n| {
            for warning in &n.warnings {
                write_err(&format!("notify-send: warning: {warning}\n"));
            }
            run(&n).map_err(|msg| format!("Failed to show notification: {msg}"))
        }),
        Parsed::Help => {
            write_out(cli::USAGE);
            Ok(())
        }
        Parsed::Version => {
            write_out(&format!("notify-send {}\n", env!("CARGO_PKG_VERSION")));
            Ok(())
        }
    });
    match result {
        Ok(()) => 0,
        Err(msg) => {
            write_err(&format!("{msg}\n"));
            EXIT_FAILURE
        }
    }
}

fn run(n: &Notification) -> Result<(), String> {
    // Before the first notification, or Windows names the app after its AUMID.
    app::register(&n.app).map_err(|e| format!("cannot register the app: {e}"))?;
    let toast = Toast::new(n).map_err(|e| e.to_string())?;
    let setting = toast.setting(&n.app.name);
    if let Setting::Off(why) = &setting {
        // A notification that does not show cannot be waited for. Otherwise,
        // the user turned notifications off, and scripts should keep working.
        if n.wait {
            return Err(why.clone());
        }
        write_err(&format!("notify-send: warning: {why}\n"));
    }
    toast.show().map_err(|e| e.to_string())?;
    // Windows tells whether it shows an app's notifications only once it has
    // shown one. If it dropped this one, a wait would never end. (One that is
    // waited for expires two seconds after it is shown at the soonest, so it
    // cannot have expired yet.)
    if n.wait && setting == Setting::Unknown && !toast.is_held() {
        return Err("Windows did not show it; notifications may be turned off in Settings > System > Notifications".into());
    }
    if n.print_id {
        write_out(&format!("{}\n", n.id));
    }
    if !n.wait {
        return Ok(());
    }
    // The messages are those of libnotify's notify-send.
    match toast.wait(n.wait_timeout) {
        Waited::Action(name) => write_out(&format!("{name}\n")),
        Waited::Closed => {}
        Waited::TimedOut => write_err("Wait timeout expired\n"),
        Waited::Interrupted => write_err("Wait cancelled, closing notification\n"),
        Waited::Failed(e) => return Err(e.to_string()),
    }
    Ok(())
}

/// Writes to stdout. Errors are ignored: if the reader is gone
/// (`notify-send -p ... | ...` after the reader quit), there is nobody left to tell.
fn write_out(text: &str) {
    let mut out = io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

/// Writes to stderr, ignoring errors like `write_out`.
fn write_err(text: &str) {
    let _ = io::stderr().lock().write_all(text.as_bytes());
}
