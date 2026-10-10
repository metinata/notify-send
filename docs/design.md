# Design

This document explains how notify-send for Windows maps the Linux command onto
Windows notifications, and why it is built the way it is. Where it says that
Windows behaves a certain way, that was measured on Windows 11 (build 26300)
with the probes described under [Testing](#testing).

## Goals

- **Compatibility with libnotify's `notify-send(1)`.** The same options, the same
  output and the same exit codes, so scripts and habits carry over from Linux and
  WSL.
- **A single, small, self-contained executable.** It needs no runtime and runs no
  service. The C runtime is linked statically (`.cargo/config.toml`), so the release
  build depends only on DLLs that ship with Windows.
- **Native notifications.** Notifications look and behave like those of any other
  app: they appear in the notification center, follow Do not disturb, and can be
  turned off per app in Settings.
- **Nothing to set up.** No administrator rights, no Start menu shortcut, no
  installer step: the first run registers what Windows needs.

Out of scope: a notification daemon or other background process, rich notification
layouts beyond what `notify-send` can express, and notifications to other machines.

## Reference behavior

The reference is `tools/notify-send.c` in
[libnotify](https://gitlab.gnome.org/GNOME/libnotify) 0.8.8, the version that
distributions ship, which parses its options with GLib's `GOptionContext`
(`glib/goption.c`). Details that matter for compatibility:

- Options can come before or after the summary and body, and `--` ends them. A long
  option takes its value as `--name=value` or as the next argument; a flag given a
  value (`--wait=yes`) is set and the value ignored.
- A short option that takes a value takes the next argument, also inside a group of
  short options: `-ap Backup` is `-a Backup -p`. A value is never attached, so
  `-ucritical` is an error, and a second option that takes a value in the same
  group is "Error parsing option -X". This is not POSIX `getopt`, which is why
  notify-send has its own parser rather than a getopt-style library.
- `-h` is `--hint`, not help: help is `-?`, `--help` or `--help-all`. `-v` prints
  the version.
- `-t` and `-r` take integers the way GLib does: with C's `strtol` in base 0, so
  `0x10` is 16 and `010` is 8, and the whole value must be a number.
- The body, but not the summary, goes through `g_strcompress`: `\n`, `\t` and the
  other C escapes are replaced, `\\` is a backslash, octal escapes produce bytes, and
  the result must be valid UTF-8.
- `-A [NAME=]Text` adds an action. The name and label are trimmed; an action without
  a name is named after its position among the `-A` options, from 0; one with an
  empty name or label is left out. Actions that were added imply `--wait`. (The
  development version of libnotify counts from 1, which no release does yet.)
- Hints are `TYPE:NAME:VALUE`, split at the first two colons. `int` and `double`
  values must start with a digit, `byte` values are decimal or `0x` hex up to 255,
  and `boolean` is true for `true` or a non-zero number. A bad hint stops the
  notification, with exit status 1. Hints are applied after `-u` and `-e`, so they
  override them.
- `-p` prints the ID after the notification is shown. With `--wait`, the chosen
  action's name is printed when the user picks one. Ctrl+C prints "Wait cancelled,
  closing notification", closes the notification and exits with status 0. With
  `-t`, the wait ends after that time with "Wait timeout expired", also status 0.
- Errors and these messages are printed as the message alone, without the program's
  name, and notify-send prints them word for word: "No summary specified.",
  "Invalid number of options.", "Unknown option -x", "Missing argument for -t",
  "Cannot parse integer value “abc” for -t", "Failed to show notification: ...".

On Linux, a notification server (GNOME Shell, KDE Plasma, dunst, ...) decides how
notifications look and behave, and servers differ: GNOME ignores `-t`, for example.
On Windows, notify-send plays both parts: it parses the options like libnotify, and
decides what a server would.

## Windows notifications

Windows shows notifications ("toasts") through `Windows.UI.Notifications`. A toast
is an XML document (`src/toast.rs`) shown by a `ToastNotifier` for an
*AppUserModelID* (AUMID), the identity of an app.

### The app identity

Packaged apps get an AUMID from their package. A plain executable has none, and the
usual way to get one is a Start menu shortcut that carries it. notify-send registers
the AUMID in the registry instead, for the current user, which needs neither a
shortcut nor administrator rights (`src/app.rs`):

```text
HKEY_CURRENT_USER\Software\Classes\AppUserModelId\metinata.notify-send
    DisplayName  REG_SZ  notify-send
    IconUri      REG_SZ  C:\Users\...\AppData\Local\notify-send\icon-4879799b.png
```

Windows takes the app's name and icon for the notification's header, the
notification center and Settings from these values. Measured behavior:

- The key must exist before the AUMID's first notification. For an AUMID it does not
  know, Windows shows the AUMID itself as the name, and keeps showing it after the key
  is created. notify-send therefore registers before every notification, writing
  values only when they change.
- Once the key exists, changes to `DisplayName` and `IconUri` show at once.
- `IconUri` takes a path to a PNG or ICO file. notify-send writes its own icon (a PNG
  embedded in the executable) to `%LOCALAPPDATA%\notify-send`, under a name that
  changes with its contents. Windows may keep showing a cached image for an
  unchanged path, but picks up a new path at once. Icons that other versions wrote
  stay, since app names registered by those versions still point to them. If the
  icon cannot be written, the registered one stays.
- A `DisplayName` that starts with `@` is read as a reference to a string in a
  module (`@module,-id`), as Windows' own entries use it. An app name that starts
  with `@` gets a zero-width space in front, so that it shows as written.

Each app name (`-a`) gets an AUMID of its own: `metinata.notify-send` for the
default name `notify-send`, and `metinata.notify-send.<hash>` for others, where
the hash is the 64-bit FNV-1a hash of the name (`src/identity.rs`). An AUMID may
not contain spaces and is at most 128 characters, so names cannot be used as they
are. Windows keeps the user's settings for each app (notifications on or off,
sound, priority) by AUMID, so this function must never change; a unit test pins its
values.

With an AUMID per app name, notifications show the name given with `-a`, are grouped
by it in the notification center, and can be turned off per app in Settings →
System → Notifications, much like on Linux. `-n` sets the app's icon, which is stored
as its `IconUri`.

### Mapping the options

| Option | Windows |
|---|---|
| summary, body | The toast's first and second `<text>` |
| `-u low` | `SuppressPopup`: straight to the notification center, as GNOME and KDE do |
| `-u normal` | A plain toast |
| `-u critical` | The `reminder` scenario, which stays on screen until dismissed, and `Priority = High` |
| `-t TIME` | `ExpirationTime`: out of the notification center after `TIME` ms (see below for the banner). Longer than a banner stays up (the user's setting, 5 seconds by default), it also gets `duration="long"`, about 25 seconds, unless the setting is 25 seconds or more |
| `-t 0` | Never expires: the `reminder` scenario, as for critical |
| `-a NAME` | The AUMID of `NAME` (see above) |
| `-i FILE` | `<image placement="appLogoOverride">` |
| `-n FILE` | The app's `IconUri` |
| `-c TYPE` | Accepted and ignored; Windows has no notification categories |
| `-e` | Removed with its banner: `ExpirationTime` a little after the banner goes, or at `-t` if that is sooner, and `ExpiresOnReboot` |
| `-p` | Prints the ID, which is the toast's `Tag` |
| `-r ID` | The toast's `Tag` is `ID`; the toast with that tag is removed first. A negative ID is out of range, as in libnotify: a warning, and a new notification |
| `-w` | Waits for the toast's events |
| `-A NAME=Text` | A button with `content="Text"` and its position as `arguments`; the `default` action is a click on the toast (`launch="default"`) |

A toast in the `reminder` scenario stays on screen only if it has a button;
without one, Windows silently shows it as a normal toast. A critical toast without
actions therefore gets the system's Dismiss button
(`<action activationType="system" arguments="dismiss" content=""/>`), which Windows
labels in the user's language. A toast with buttons of its own stays without it.

Toasts with actions are shown in the `reminder` scenario too, so they stay on screen
until the user picks an action or closes them, and they pop up even with `-u low`.
A script is waiting for the answer, and on Windows a toast that times out goes to
the notification center, where the user may not look for it. Windows shows at most
five buttons; notify-send refuses more rather than drop some. Toasts with actions
also get `ExpiresOnReboot`, since their buttons do nothing once the wait is over
(see [Waiting](#waiting)).

An expired toast leaves the notification center at once, but Windows does not take
its banner off the screen: the banner stays up for its usual time, or, in the
`reminder` scenario, until it is dismissed. Without `--wait`, notify-send has
exited by then and cannot take it down, so `-t` cannot make a banner go sooner;
`duration="long"` at least keeps it up for as long as `-t` asks, up to 25 seconds.
A critical toast with `-t` therefore stays on screen until it is dismissed, and
only leaves the notification center at `TIME`. With `--wait`, the wait takes the
toast down when `-t` passes.

`Priority` came with Windows 10 1703 and `ExpiresOnReboot` with 1903. On older
versions, which some long-term servicing editions still run, setting them fails;
the toast is then shown without them, rather than not at all.

Hints that Windows has a counterpart for:

| Hint | Windows |
|---|---|
| `urgency` (byte) | Overrides `-u`: 0 low, 1 normal, 2 critical |
| `transient` (boolean) | Overrides `-e`, also to turn it off |
| `suppress-sound` (boolean) | `<audio silent="true"/>` |
| `sound-name` (string) | `<audio src="ms-winsoundevent:...">`: `message-new-instant` → IM, `message-new-email` → Mail, `alarm-clock-elapsed` → Reminder, and Windows names as they are (`Mail`, `ms-winsoundevent:Notification.SMS`). Other names keep the default sound |
| `image-path` (string) | Like `-i`, and wins over it |
| `value` (int) | A progress bar (`<progress value="..." status=""/>`), as dunst and xfce4-notifyd show it |

Other hints are accepted and ignored, as a notification server ignores hints it
does not support. libnotify parses `variant` values as GVariant text and refuses
text it cannot parse; here they are not checked, since no hint with a counterpart
needs one.

### Text

The body is plain text: Windows does not render the markup (`<b>`, `<i>`, `<a>`)
that some Linux servers do, so tags show as written. Characters that XML does not
allow (most control characters) are left out. Windows stores line breaks as CRLF.

### Images

`-i` and `-n` take a file path, relative to the current directory or absolute, or a
`file://` URI (also `file://localhost/...`, in any case). On Linux they may also
name an icon from the icon theme, such as `dialog-information` or
`org.gnome.Nautilus`; Windows has no icon theme, so such names are ignored. A value
that looks like a path (it has a slash, a backslash or a colon, or an image file
extension) but names no file gets a warning on stderr, and the notification is shown
without it. Windows does the same with an image it cannot load.

Only files on a drive of this computer are used. notify-send first works out the
absolute path as Windows will open it, with `std::path::absolute`, which only
rewrites the text; that also turns a device name such as `NUL` into a `\\.\` path.
It then refuses, with a warning and before anything touches the path, every path
that is not on a drive: other computers (`\\server\share\...`, also spelled
`//server/...`, `\\?\UNC\...` or, for WebDAV, `\\server@SSL\...`, and
`file://server/...`) and devices (`\\.\...`, `\\?\GLOBALROOT\...`). Drive letters
mapped to network shares are refused too; Windows knows them from the mapping
(`GetDriveTypeW`), without contacting the server. Looking up a file on another
computer makes Windows connect to it and sign in with the user's credentials, so a
value taken from a commit message or a model's output could hand its author a
crackable hash; `-n` would even store the path as the app's icon, for Windows to
fetch again later.

The image is passed to Windows as a `file:` URI. ASCII characters outside the
unreserved set are percent-encoded, so `#` and `%` in a file name stay part of it.
Other characters are left as they are: Windows does not find an image whose URI has
them percent-encoded as UTF-8 (`%C4%9F` for `ğ`), but does with the characters
themselves.

### Replacing a notification

A toast's `Tag` and `Group` identify it within its app. notify-send uses the ID as
the tag and `notify-send` as the group, so a toast with the same ID replaces the
earlier one, which is what `-r` does on Linux.

When the toast being replaced is on screen, Windows takes it down and puts the new
one in the notification center without showing it. On Linux, the replaced
notification shows. notify-send therefore removes the old toast from the history
first (`ToastNotificationHistory.Remove`), and shows the new one as a new toast.

IDs on Linux come from the notification server, which counts up. Without a server,
notify-send picks a random ID from 1 to 2³¹−1 for a new notification. Since the tag
belongs to an app, `-r` only replaces a notification of the same app name.

### Waiting

`--wait`, and `-A`, which implies it, wait for the toast's events: `Activated` (a
click on a button or on the toast, with the button's `arguments` or the toast's
`launch`), `Dismissed` (with a reason: the user closed it, the application hid it,
or its banner timed out) and `Failed`. These events reach a process without package
identity too, as long as it runs, so a wait needs no COM activator. They arrive on
threads of Windows' thread pool and go to the main thread through a channel
(`src/show.rs`).

| Event | Result |
|---|---|
| A button, or the toast with a `default` action | Prints the action's name, takes the toast down, exits with 0 |
| A click on the toast without a `default` action | Exits with 0 (Windows removes the toast) |
| Closed by the user, or replaced | Exits with 0 |
| The banner timed out | Exits with 0; the toast is taken down if it has buttons or is transient, and stays in the notification center otherwise |
| `-t` passed | "Wait timeout expired", takes the toast down, exits with 0 |
| Ctrl+C or Ctrl+Break | "Wait cancelled, closing notification", takes the toast down, exits with 0 |
| The console window closes | Takes the toast down; Windows then ends notify-send |
| `Failed` | "Failed to show notification: ...", exits with 1 |

Taking a toast down means `ToastNotifier.Hide`, which takes it off the screen, and
removing it from the history, which takes it out of the notification center. It
happens whenever the wait ends without the toast going away by itself, so that no
buttons outlive the wait: clicked after notify-send exited, they would do nothing.
A toast that is waited for with `-t` expires two seconds after the wait timeout.
The wait takes it down itself, as expiring would not take its banner off the
screen, and the expiry is only for when notify-send is ended before that.

Buttons report their position as their `arguments`, and the toast reports `default`
when it is clicked; notify-send looks the action's name up from that. Action names
therefore never appear in the XML, and no name can pass for another, or for a click
on the toast, whatever characters it contains.

A banner that times out ends the wait, as on most Linux servers. Toasts that stay on
screen (critical, `-t 0`, or with actions) do not time out, except when Windows
holds banners back: under Do not disturb, or with banners turned off for the app,
it reports them as timed out after the usual banner time, and a question then ends
without an answer. Low-urgency toasts never show a banner, and Windows reports them
as timed out at once, so `-u low --wait` returns immediately. On GitHub's Windows
runners, it reports plain toasts as gone at once too, while questions stay.

The console control handler runs on a thread Windows creates for each event. It
takes the toast down first, then, for Ctrl+C and Ctrl+Break, tells the main thread,
which prints the message and exits: a shell that captures notify-send's output may
end it as soon as Ctrl+C is pressed, and when the console window closes, Windows
ends the process as soon as the handler returns. A second Ctrl+C is left to Windows,
which ends the process, in case the main thread is stuck in a call to the
notification platform. Windows does not report signing out or shutting down to a
console program that has loaded user32.dll, as notify-send has, so a question still
open then stays until the next restart (`ExpiresOnReboot`).

### When notifications are off

`ToastNotifier.Setting` says whether Windows will show the app's notifications:
they may be turned off for the app or for the user in Settings, or by group policy.
Then notify-send prints a warning and exits with 0, as a Linux server accepts a
notification for an app the user muted, so that scripts keep working. A wait would
never end, so with `--wait` or actions it exits with 1 instead. Do not disturb is
not such a case: notifications then still go to the notification center.

`Setting` fails for an app that has not shown a notification yet, as every new app
name has not. When it fails and notify-send is to wait, it checks after showing the
notification that Windows holds it (`ToastNotificationHistory`), since Windows
drops the notifications of an app that is turned off, and a wait for one would never
end. It cannot have expired by then: a notification that is waited for expires two
seconds after it is shown at the soonest.

## Program flow

1. Parse the arguments (`src/cli.rs`), following GLib's rules, then the hints
   (`src/hints.rs`) and the body's escapes (`src/escape.rs`).
2. Work out the notification (`src/notification.rs`): the app, the toast XML, the
   ID, how long it lasts, and whether to wait. This step is pure, apart from looking
   up image files, and is unit tested.
3. Register the app's AUMID (`src/app.rs`).
4. Create the toast and its notifier (`src/show.rs`). With `--wait`, subscribe to
   the toast's events and the console's control events before it is shown.
5. Check `ToastNotifier.Setting`, and show the toast; with `-r`, remove the old one
   first. Print the ID with `-p`.
6. With `--wait`, wait for the first event, the timeout, or Ctrl+C.

Output goes through `write_all` with errors ignored, so `notify-send -p` into a pipe
whose reader has quit does not panic. Action names and the ID are written in UTF-8,
as on Linux.

## Security

The text of a notification often comes from somewhere else: a commit message, a
file name, a web page, a model's output. Someone who controls such a value must not
be able to do more than change what the notification says.

- **The toast's structure is fixed.** Every value that goes into the toast XML is
  escaped (`src/toast.rs`), and characters that XML does not allow are left out. A
  pre-release review fed every Unicode character, attempts to close elements and
  attributes, DOCTYPE and CDATA payloads and 100,000-character values through every
  field, and Windows always parsed the same elements and attributes: no value can
  add a button, a link, a protocol activation or an image. Action names do not go
  into the XML at all.
- **No other computers.** Images that are not files on a drive of this computer are
  refused before they are touched (see [Images](#images)), and an app name cannot
  make Windows load a module (see [The app identity](#the-app-identity)). A symbolic
  link on a local drive that points to another computer is still followed; making
  one takes access to the file system already.
- **State stays small and in place.** Registry writes go only to
  `HKCU\Software\Classes\AppUserModelId\metinata.notify-send*`, under a key derived
  from a hash of the app name, so the name never becomes part of a key path. Files
  are written only to `%LOCALAPPDATA%\notify-send`.
- **Options in the text.** As on Linux, an argument that starts with `-` is an
  option, wherever it is. Scripts that pass text from elsewhere should put `--` in
  front of it; the README says so.

## Exit status

| Code | Meaning |
|---|---|
| `0` | The notification was shown, or notifications are turned off without `--wait`. With `--wait`, the wait ended for any reason other than a failure, including Ctrl+C and the timeout |
| `1` | Invalid arguments, a bad hint, more than five actions, or the notification could not be shown |

## Testing

- **Unit tests** cover argument parsing with GLib's rules (`src/cli.rs`), the body's
  escapes (`src/escape.rs`), hints (`src/hints.rs`), the AUMID and its stability and
  display names (`src/app.rs`), the toast XML and file URIs (`src/toast.rs`), and
  what each option turns into, including which images are refused
  (`src/notification.rs`).
- **Integration tests** (`tests/cli.rs`) run the built executable. Notifications
  they show use the app name "notify-send tests" and, where possible, low urgency,
  so that they go straight to the notification center. The tests read what Windows
  received through `ToastNotificationManager.History`, which needs no user
  interaction, and remove what they showed. They cover showing, registering the app,
  `-r`, `-t`, the body's escapes, icons, refusing images on other computers, the
  wait timeout, Ctrl+Break during a wait, libnotify's error messages, and that the
  executable does not need the Visual C++ runtime.

Whether Windows can show notifications at all is checked once, by showing one
directly through WinRT rather than through notify-send, so that a broken
notify-send fails the tests instead of skipping them. Where Windows cannot (in a
service, for example), the tests that need notifications are skipped, with a warning
written straight to stderr, which `cargo test` does not capture.

Clicking a toast cannot be automated: UI Automation does not reach toasts. Clicks
on buttons and on the toast, closing it, and closing the console window during a
wait are tested by hand, as described in `CONTRIBUTING.md`.

The measurements this design relies on (the registry AUMID, events without package
identity, which scenarios stay on screen, expiration, replacing, image URIs) were
made with a small probe program and screenshots, and are recorded above.

## Future work

- Update a replaced notification in place while it is on screen, with toast data
  binding (`NotificationData` and `ToastNotifier.Update`), instead of showing it
  again; this would make frequent progress updates smoother.
- Render a subset of the body markup that Linux servers accept (`<b>`, `<i>`,
  `<a href>`), by stripping tags and decoding entities when the body parses as
  markup.
- Parse `variant` hints, and apply the ones with a counterpart.
- `--id-fd` and `--selected-action-fd`, for file descriptors 1 and 2.
- A Scoop package, publishing to crates.io, and code signing for release binaries.
