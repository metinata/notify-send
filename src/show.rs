//! Showing a notification through the Windows notification platform
//! (`Windows.UI.Notifications`), and waiting for what happens to it.
//!
//! The toast's events (activated, dismissed, failed) reach a process without
//! package identity too, as long as it runs, so a wait needs no COM server or
//! Start menu shortcut.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{DateTime, IReference, PropertyValue, TypedEventHandler};
use windows::UI::Notifications::{
    NotificationSetting, ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs,
    ToastFailedEventArgs, ToastNotification, ToastNotificationManager, ToastNotificationPriority,
    ToastNotifier,
};
use windows::core::{Error, HSTRING, IInspectable, Interface, Result};

use crate::console;
use crate::identity::GROUP;
use crate::notification::Notification;
use crate::toast::CLICK;

/// Whether Windows shows the app's notifications.
#[derive(Debug, PartialEq, Eq)]
pub enum Setting {
    On,
    /// Turned off, for this reason.
    Off(String),
    /// Windows knows an app's setting only once it has shown one of its
    /// notifications.
    Unknown,
}

/// How a wait ended.
#[derive(Debug)]
pub enum Waited {
    /// The user chose the action with this name.
    Action(String),
    /// The notification went away without an action: the user closed it, its
    /// banner timed out, or another notification replaced it.
    Closed,
    /// The wait timeout passed.
    TimedOut,
    /// Ctrl+C or Ctrl+Break.
    Interrupted,
    /// Windows could not show the notification.
    Failed(Error),
}

enum Event {
    Activated(String),
    Dismissed(ToastDismissalReason),
    Failed(Error),
    Interrupted,
}

/// What it takes to take a notification down. The console handler gets a
/// copy, as it takes the notification down itself.
#[derive(Clone)]
struct Remover {
    notifier: ToastNotifier,
    toast: ToastNotification,
    tag: HSTRING,
    aumid: HSTRING,
}

impl Remover {
    /// Takes the notification off the screen and out of the notification center.
    fn remove(&self) {
        let _ = self.notifier.Hide(&self.toast);
        if let Ok(history) = ToastNotificationManager::History() {
            let _ = history.RemoveGroupedTagWithId(&self.tag, &HSTRING::from(GROUP), &self.aumid);
        }
    }
}

pub struct Toast {
    remover: Remover,
    /// Replaces the notification with the same tag (`-r`).
    replace: bool,
    /// The names of the buttons' actions, by position.
    buttons: Vec<String>,
    /// Whether a click on the notification is the `default` action.
    clickable: bool,
    /// Take it down when its banner times out: it is transient, or has
    /// buttons that nobody listens to once the wait is over.
    remove_when_hidden: bool,
    events: Option<Receiver<Event>>,
}

impl Toast {
    /// Creates the notification without showing it. If it is to be waited
    /// for, this already subscribes to its events and to Ctrl+C, so that
    /// nothing is missed once it shows.
    pub fn new(n: &Notification) -> Result<Self> {
        let content = XmlDocument::new()?;
        content.LoadXml(&HSTRING::from(&n.xml))?;
        let toast = ToastNotification::CreateToastNotification(&content)?;
        let tag = HSTRING::from(n.id.to_string());
        toast.SetTag(&tag)?;
        toast.SetGroup(&HSTRING::from(GROUP))?;
        if !n.popup {
            toast.SetSuppressPopup(true)?;
        }
        if let Some(after) = n.expires {
            toast.SetExpirationTime(&date_time(SystemTime::now() + after)?)?;
        }
        // Older versions of Windows 10 do not have these (Priority came in
        // 1703, ExpiresOnReboot in 1903); the notification is shown without.
        if n.high_priority {
            let _ = toast.SetPriority(ToastNotificationPriority::High);
        }
        if n.expires_on_reboot {
            let _ = toast.SetExpiresOnReboot(true);
        }
        let aumid = HSTRING::from(&n.app.aumid);
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&aumid)?;
        let remover = Remover {
            notifier,
            toast,
            tag,
            aumid,
        };
        let events = if n.wait {
            Some(subscribe(&remover)?)
        } else {
            None
        };
        Ok(Self {
            remover,
            replace: n.replace,
            buttons: n.buttons.clone(),
            clickable: n.clickable,
            remove_when_hidden: n.transient || !n.buttons.is_empty(),
            events,
        })
    }

    /// Whether Windows shows the app's notifications. Do not disturb does not
    /// turn them off: they still go to the notification center.
    pub fn setting(&self, app_name: &str) -> Setting {
        match self.remover.notifier.Setting() {
            Ok(NotificationSetting::DisabledForApplication) => Setting::Off(format!(
                "notifications from \"{app_name}\" are turned off in Settings > System > Notifications"
            )),
            Ok(NotificationSetting::DisabledForUser) => Setting::Off(
                "notifications are turned off in Settings > System > Notifications".into(),
            ),
            Ok(NotificationSetting::DisabledByGroupPolicy) => {
                Setting::Off("notifications are turned off by group policy".into())
            }
            Ok(_) => Setting::On,
            Err(_) => Setting::Unknown,
        }
    }

    pub fn show(&self) -> Result<()> {
        let r = &self.remover;
        if self.replace {
            // Showing a notification with the tag of one that is on screen
            // takes that one down and puts the new one in the notification
            // center without showing it. Removing the old one first makes the
            // new one show, as a replaced notification does on Linux.
            let history = ToastNotificationManager::History()?;
            let _ = history.RemoveGroupedTagWithId(&r.tag, &HSTRING::from(GROUP), &r.aumid);
        }
        r.notifier.Show(&r.toast)
    }

    /// Whether Windows holds the notification, on screen or in the
    /// notification center; it drops those of an app whose notifications are
    /// turned off. Checked for up to a second, since Windows records a
    /// notification a moment after it is shown.
    pub fn is_held(&self) -> bool {
        let r = &self.remover;
        let end = Instant::now() + Duration::from_secs(1);
        loop {
            let held = ToastNotificationManager::History()
                .and_then(|history| history.GetHistoryWithId(&r.aumid))
                .map(|list| {
                    list.into_iter()
                        .any(|toast| toast.Tag().is_ok_and(|tag| tag == r.tag))
                });
            match held {
                // If the history cannot be read, there is no telling.
                Ok(true) | Err(_) => return true,
                Ok(false) if Instant::now() >= end => return false,
                Ok(false) => sleep(Duration::from_millis(50)),
            }
        }
    }

    /// Waits until the user chooses an action or closes the notification,
    /// its banner times out, Ctrl+C is pressed, or `timeout` passes. Unless
    /// the notification goes away by itself, it is taken down, so that its
    /// buttons do not outlive the wait.
    pub fn wait(&self, timeout: Option<Duration>) -> Waited {
        let Some(events) = &self.events else {
            return Waited::Closed;
        };
        let event = match timeout {
            Some(timeout) => events.recv_timeout(timeout),
            None => events.recv().map_err(RecvTimeoutError::from),
        };
        match event {
            Ok(Event::Activated(arguments)) => {
                self.remover.remove();
                match self.chosen(&arguments) {
                    Some(name) => Waited::Action(name.to_owned()),
                    // A click on the notification without a default action, or
                    // on the system's Dismiss button.
                    None => Waited::Closed,
                }
            }
            Ok(Event::Dismissed(reason)) => {
                // When Windows holds banners back (Do not disturb, banners
                // turned off), it reports this after the usual banner time.
                if reason == ToastDismissalReason::TimedOut && self.remove_when_hidden {
                    self.remover.remove();
                }
                Waited::Closed
            }
            Ok(Event::Failed(error)) => Waited::Failed(error),
            Ok(Event::Interrupted) => {
                self.remover.remove();
                Waited::Interrupted
            }
            Err(RecvTimeoutError::Timeout) => {
                self.remover.remove();
                Waited::TimedOut
            }
            // The console handler keeps a sender, so this does not happen.
            Err(RecvTimeoutError::Disconnected) => Waited::Closed,
        }
    }

    /// The name of the action that Windows reported a click on: buttons
    /// report their position, and the notification itself `CLICK`.
    fn chosen(&self, arguments: &str) -> Option<&str> {
        if self.clickable && arguments == CLICK {
            return Some(CLICK);
        }
        let position: usize = arguments.parse().ok()?;
        self.buttons.get(position).map(String::as_str)
    }
}

/// Sends the toast's events, and Ctrl+C, to the returned receiver. They
/// arrive on threads of Windows' thread pool and the console.
fn subscribe(remover: &Remover) -> Result<Receiver<Event>> {
    let (sender, events) = mpsc::channel();
    let send = sender.clone();
    remover
        .toast
        .Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, args| {
                let args: ToastActivatedEventArgs = args.ok()?.cast()?;
                let _ = send.send(Event::Activated(args.Arguments()?.to_string()));
                Ok(())
            },
        ))?;
    let send = sender.clone();
    remover.toast.Dismissed(&TypedEventHandler::<
        ToastNotification,
        ToastDismissedEventArgs,
    >::new(move |_, args| {
        let _ = send.send(Event::Dismissed(args.ok()?.Reason()?));
        Ok(())
    }))?;
    let send = sender.clone();
    remover.toast.Failed(
        &TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, args| {
            let _ = send.send(Event::Failed(args.ok()?.ErrorCode()?.into()));
            Ok(())
        }),
    )?;
    let remover = remover.clone();
    console::on_event(move |event| {
        // Taken down here, first: when the console closes, Windows ends
        // notify-send as soon as this returns, and on Ctrl+C a shell that
        // captures notify-send's output may end it right away too.
        remover.remove();
        if event == console::Event::Interrupt {
            let _ = sender.send(Event::Interrupted);
        }
    });
    Ok(events)
}

/// A WinRT `DateTime`, which counts 100-nanosecond intervals since 1601 (UTC).
fn date_time(time: SystemTime) -> Result<IReference<DateTime>> {
    const UNIX_EPOCH_IN_TICKS: i64 = 116_444_736_000_000_000;
    let since_unix_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let ticks = i64::try_from(since_unix_epoch.as_nanos() / 100)
        .unwrap_or(i64::MAX - UNIX_EPOCH_IN_TICKS)
        .saturating_add(UNIX_EPOCH_IN_TICKS);
    PropertyValue::CreateDateTime(DateTime {
        UniversalTime: ticks,
    })?
    .cast()
}
