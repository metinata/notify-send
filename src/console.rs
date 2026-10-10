//! Console events while notify-send waits: Ctrl+C and Ctrl+Break, and the
//! console window being closed.
//!
//! Signing out and shutting down are not among them: Windows does not send
//! those events to a process that has loaded user32.dll, as notify-send has.
//! Notifications with buttons expire on reboot for that reason (see
//! `src/notification.rs`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use windows::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, SetConsoleCtrlHandler,
};
use windows::core::BOOL;

/// What happened to the console.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Ctrl+C or Ctrl+Break.
    Interrupt,
    /// The console window is being closed. Windows ends the process as soon
    /// as the handler returns.
    Close,
}

type Handler = Box<dyn Fn(Event) + Send>;

static HANDLER: Mutex<Option<Handler>> = Mutex::new(None);

/// Whether Ctrl+C has been handled already.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn on_console_event(ctrl_type: u32) -> BOOL {
    let event = match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => Event::Interrupt,
        CTRL_CLOSE_EVENT => Event::Close,
        _ => return false.into(),
    };
    // A second Ctrl+C is left to Windows, which ends the process, in case the
    // main thread is stuck in a call to the notification platform and cannot
    // act on the first.
    if event == Event::Interrupt && INTERRUPTED.swap(true, Ordering::SeqCst) {
        return false.into();
    }
    if let Some(handler) = HANDLER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        handler(event);
    }
    true.into()
}

/// Calls `handler` on the console's events, on a thread that Windows creates
/// for each event. Without a console, nothing happens.
pub fn on_event(handler: impl Fn(Event) + Send + 'static) {
    *HANDLER.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(handler));
    // Best effort: without the handler, Ctrl+C simply ends notify-send, and
    // the notification stays.
    // SAFETY: `on_console_event` matches PHANDLER_ROUTINE and only touches the statics above.
    let _ = unsafe { SetConsoleCtrlHandler(Some(on_console_event), true) };
}
