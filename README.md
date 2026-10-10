<p align="center">
  <img src="docs/assets/logo.svg" width="112" height="112" alt="">
</p>

<h1 align="center">notify-send for Windows</h1>

<p align="center">
  The Linux <code>notify-send</code> command, native on Windows.<br>
  Show desktop notifications from scripts, builds and AI agents, with buttons,<br>
  progress bars and the same options as on Linux.
</p>

<p align="center">
  <a href="https://github.com/metinata/notify-send/actions/workflows/ci.yml"><img src="https://github.com/metinata/notify-send/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/metinata/notify-send/releases/latest"><img src="https://img.shields.io/github/v/release/metinata/notify-send" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6" alt="Windows 10 and 11">
</p>

<p align="center">
  <img src="docs/assets/demo.svg" width="680" alt="Examples in a terminal: npm test &amp;&amp; notify-send &quot;Tests passed&quot; || notify-send -u critical &quot;Tests failed&quot; tells you when the tests are done and whether they passed; winget upgrade --all; notify-send &quot;Updates finished&quot; tells you when all your apps are updated; notify-send -A yes=Deploy -A no=Cancel &quot;Deploy to production?&quot; asks a question and prints yes when you click Deploy. A Windows notification from notify-send asks: Deploy to production?, with the buttons Deploy and Cancel">
</p>

## Features

- **Same command line as Linux.** The options of libnotify's `notify-send(1)`
  behave the same way: urgency, expire time, app name, icons, hints, `-p`/`-r` to
  replace a notification, `--wait`, and `-A` buttons that print what was chosen.
  Scripts and habits carry over, also from WSL.
- **Real Windows notifications.** They appear in the notification center, follow
  Do not disturb, and each app name (`-a`) gets its own entry in Settings, where it
  can be turned off.
- **One small executable, for any shell.** A single `notify-send.exe` of about
  360 KB, with nothing else to install: no runtime, no background service, no
  administrator rights. It works the same from PowerShell, Command Prompt, Git Bash,
  WSL, scripts and scheduled tasks.
- **Buttons and progress bars.** Ask a question and act on the answer, or show a
  progress bar that a later call updates.
- **Unicode throughout.** Text in any language, emoji, and file names like
  `C:\Görseller\ikon #1.png`.

## Installation

Run this in PowerShell:

```powershell
irm https://github.com/metinata/notify-send/releases/latest/download/install.ps1 | iex
```

The [install script](install.ps1) downloads the latest release for your machine,
checks its SHA-256 checksum, puts `notify-send.exe` in
`%LOCALAPPDATA%\Programs\notify-send` and adds that folder to your `PATH`. It needs
no administrator rights. Run it again to update.

The checksum comes from the same release, so it guards against a damaged
download; for the release itself, the script trusts GitHub. To check that a
release was built from this repository by its release workflow, verify its
attestation, as shown under "Other ways to install".

<details>
<summary><b>Other ways to install</b></summary>

#### Download

Download the zip for your machine from the
[latest release](https://github.com/metinata/notify-send/releases/latest),
`x86_64` for most PCs or `aarch64` for Windows on ARM, and extract `notify-send.exe`
into a folder on your `PATH`.

Each release comes with SHA-256 checksums and
[build provenance attestations](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations/using-artifact-attestations-to-establish-provenance-for-builds).
To check that a download was built from this repository by its release workflow:

```console
> gh attestation verify notify-send-x86_64-pc-windows-msvc.zip --repo metinata/notify-send
```

#### Build from source

With [Rust](https://rustup.rs) 1.85 or newer and the MSVC build tools
(Visual Studio's "Desktop development with C++" workload):

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo install --git https://github.com/metinata/notify-send --locked
```

`cargo install` does not read the repository's `.cargo/config.toml`, so
`RUSTFLAGS` links the C runtime statically instead; without it, the program
needs the Visual C++ Redistributable.

A winget package is planned.

</details>

<details>
<summary><b>Uninstall</b></summary>

If you used the install script, delete its folder and take it off your `PATH`:

```powershell
$dir = "$env:LOCALAPPDATA\Programs\notify-send"
Remove-Item -Recurse $dir
$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
$path = $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
$key.SetValue('Path', (($path -split ';') -ne $dir) -join ';', 'ExpandString')
```

notify-send also registers each app name it shows notifications for, so that
Windows can show its name and icon (see [How it works](#how-it-works)). To remove
those registrations and notify-send's icon:

```powershell
Get-ChildItem HKCU:\Software\Classes\AppUserModelId |
    Where-Object PSChildName -like 'metinata.notify-send*' | Remove-Item -Recurse
Remove-Item -Recurse "$env:LOCALAPPDATA\notify-send"
```

</details>

## Usage

```text
notify-send [OPTION…] <SUMMARY> [BODY]
```

| Option | Effect |
|---|---|
| `-u`, `--urgency=LEVEL` | `low`, `normal` or `critical`. A low-urgency notification goes straight to the notification center without popping up, as on GNOME and KDE. A critical one stays on screen until it is dismissed. |
| `-t`, `--expire-time=TIME` | Remove the notification after `TIME` milliseconds. Without `-w`, its banner may stay up longer (see [Differences from Linux](#differences-from-linux)). `0` keeps it on screen until it is dismissed. |
| `-a`, `--app-name=APP_NAME` | Show the notification as coming from this app: under its name, grouped with the app's other notifications, and with its own switch in Settings → System → Notifications. |
| `-i`, `--icon=ICON` | An image file on this computer (PNG, JPEG, GIF) to show in the notification. |
| `-n`, `--app-icon=ICON` | An image file on this computer to use as the app's icon. |
| `-c`, `--category=TYPE[,TYPE...]` | Accepted for compatibility, and not used. |
| `-e`, `--transient` | Remove the notification when its banner goes away, instead of keeping it in the notification center. |
| `-h`, `--hint=TYPE:NAME:VALUE` | Extra data; see [Hints](#hints). |
| `-p`, `--print-id` | Print the notification's ID. |
| `-r`, `--replace-id=REPLACE_ID` | Replace the notification with this ID. |
| `-w`, `--wait` | Wait until the notification is closed or its banner goes away. With `-t`, wait at most that long. |
| `-A`, `--action=[NAME=]Text` | Add a button labeled `Text`, and wait until the user chooses one. Its `NAME` is printed, or without one, its position, counting from 0. The name `default` stands for a click on the notification itself rather than a button. Up to 5 buttons. |
| `-?`, `--help` | Print help. |
| `-v`, `--version` | Print the version. |

Options can come before or after the text, and `--` ends them, as in
`notify-send -- "-5 °C" "Frost tonight"`. Summary and body are separate arguments,
so quote text that contains spaces. As on Linux, grouped short options take their
values from the arguments that follow: `-ap Backup` is `-a Backup -p`.

When the text comes from elsewhere, such as a commit message, a file name or a
model's output, put `--` in front of it: `notify-send -a CI -- "$title" "$message"`.
Otherwise text that starts with `-` is read as options, as on Linux.

As on Linux, the body understands backslash escapes: `\n` starts a new line, `\t` is
a tab, and `\\` is a backslash. In Windows paths in the body, double the backslashes
(`C:\\Builds`) or use forward slashes. The summary is shown as written.

### Hints

| Hint | Effect |
|---|---|
| `int:value:PERCENT` | Show a progress bar. |
| `boolean:suppress-sound:true` | Show the notification without a sound. |
| `string:sound-name:NAME` | Play a different sound: `message-new-instant`, `message-new-email` and `alarm-clock-elapsed` map to the Windows sounds for messages, mail and reminders, and Windows names work too (`Mail`, `SMS`, `ms-winsoundevent:Notification.Reminder`). |
| `string:image-path:FILE` | Like `-i`. |
| `byte:urgency:LEVEL` | Like `-u`: 0 low, 1 normal, 2 critical. |
| `boolean:transient:true` | Like `-e`. |

Other hints are accepted and ignored, as a Linux notification server ignores hints
it does not support.

### Examples

```powershell
# Show a notification
notify-send "Build finished" "All 128 tests passed"

# Get notified when a long command is done
cargo build --release; notify-send "Build finished"

# Know whether the tests passed; a failure stays on screen until dismissed
# (PowerShell 7, Command Prompt and Bash; Windows PowerShell 5.1 has no && and ||)
npm test && notify-send "Tests passed" || notify-send -u critical "Tests failed"

# Stay on screen until dismissed
notify-send -u critical "Disk almost full" "C: has 2 GB left"

# Show it under your app's name, with an image
notify-send -a "Backup" -i .\backup.png "Backup complete" "1,204 files copied"

# Show a progress bar, and update it
$id = notify-send -p -h int:value:10 "Downloading" "dataset.zip"
notify-send -r $id -h int:value:60 "Downloading" "dataset.zip"

# Ask a question, and act on the answer
$answer = notify-send -A yes=Deploy -A no=Cancel "Deploy to production?"
if ($answer -eq 'yes') { .\deploy.ps1 }
```

With [caffeinate for Windows](https://github.com/metinata/caffeinate-win), the PC
also stays awake until the command is done:

```powershell
caffeinate winget upgrade --all; notify-send "Updates installed"
```

In WSL, Windows programs run by their file name, so `notify-send.exe` works from
Linux too. To use the Linux name in scripts as well, link it into a folder on
your Linux `PATH`:

```bash
mkdir -p ~/.local/bin && ln -sf "$(command -v notify-send.exe)" ~/.local/bin/notify-send
```

### Waiting for an answer

With `-A`, notify-send shows the buttons and waits. When the user chooses one, it
prints the button's name and exits:

```console
> notify-send -A yes=Deploy -A no=Cancel "Deploy to production?"
yes
```

A notification with buttons stays on screen until the user chooses one or closes
it, because a script is waiting for the answer. If the user closes it, nothing is
printed. With `-t`, the wait ends after that time with "Wait timeout expired", and
the notification goes away. <kbd>Ctrl</kbd>+<kbd>C</kbd> ends the wait too, and
takes the notification down. When Windows holds banners back (Do not disturb, or
banners turned off for the app), the question goes away again after the usual
banner time, without an answer.

The name is printed in UTF-8. PowerShell reads a program's output in the
console's code page, so to capture names with non-ASCII characters, run
`[Console]::OutputEncoding = [Text.Encoding]::UTF8` first, or keep the names ASCII
and put the other characters in the labels: `-A "no=Hayır"`.

### Exit status

| Code | Meaning |
|---|---|
| `0` | The notification was shown. With `--wait` or `-A`, the wait has ended, also after the timeout or <kbd>Ctrl</kbd>+<kbd>C</kbd>, as on Linux. |
| `1` | Invalid arguments, more than 5 buttons, or Windows could not show the notification. |

If notifications are turned off in Settings, notify-send prints a warning and exits
with `0`, so that scripts keep working. With `--wait` or `-A`, it exits with `1`,
since there is nothing to wait for.

## How it works

notify-send shows notifications with the same Windows API as other apps
([`Windows.UI.Notifications`](https://learn.microsoft.com/uwp/api/windows.ui.notifications)).
Windows shows each notification under an app identity, and takes the app's name and
icon from the registry. So that a plain executable can have one, notify-send
registers each app name it is given, for the current user only, under
`HKEY_CURRENT_USER\Software\Classes\AppUserModelId`, and keeps its icon in
`%LOCALAPPDATA%\notify-send`. That is the only state it keeps, and it needs no
administrator rights.

See [docs/design.md](docs/design.md) for the full design.

## Differences from Linux

- The body is shown as plain text. Markup such as `<b>`, which some Linux
  notification servers render, shows as written.
- `-i` and `-n` take image files. Icon names from a Linux icon theme, such as
  `dialog-information`, are ignored, since Windows has no icon theme.
- Images must be files on a drive of this computer. Paths to other computers
  (`\\server\share\...`, `file://server/...`), mapped network drives and device
  names such as `NUL` are not used: looking a file up on another computer would
  make Windows sign in to it.
- `-t` removes the notification from the notification center after the given time,
  but Windows leaves its banner up for the usual time: 5 seconds unless changed in
  the accessibility settings, or about 25 seconds if `-t` is longer. A critical
  notification stays on screen until it is dismissed. With `-w` or buttons,
  notify-send takes the notification down itself when the time is up, so
  `notify-send -w -t 3000 "Done"` shows it for 3 seconds.
- Low-urgency notifications do not pop up, and `-u low --wait` returns at once.
- Notifications with buttons stay on screen until the user answers or closes them.
- IDs are random rather than counting up, and `-r` replaces a notification only if
  it has the same app name.
- Windows shows at most 5 buttons.
- `-c` and hints that Windows has no counterpart for are accepted and ignored.
  `variant` hints are not checked.
- `--id-fd`, `--selected-action-fd` and `--activation-token-fd` are not supported.

## Contributing

Bug reports, ideas and pull requests are welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) to get started, and [SECURITY.md](SECURITY.md)
to report a vulnerability.

## License

[MIT](LICENSE) © 2026 Metin Ata YETER
