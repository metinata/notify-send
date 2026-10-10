# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-10-10

### Added

- `notify-send` command for Windows with the options of libnotify's
  `notify-send(1)`: `-u`, `-t`, `-a`, `-i`, `-n`, `-c`, `-e`, `-h`, `-p`, `-r`,
  `-w`, `-A`, `-?`/`--help` and `-v`/`--version`. They are parsed with GLib's rules,
  grouped short options included, and errors are libnotify's messages, word for word.
- Native Windows notifications without a Start menu shortcut or administrator
  rights: each app name is registered for the current user, so notifications show
  its name and icon, are grouped by it, and can be turned off per app in Settings.
- Low-urgency notifications go straight to the notification center; critical ones,
  ones that never expire (`-t 0`) and ones with buttons stay on screen until the
  user acts on them.
- Buttons (`-A`) that print the chosen action, with `default` for a click on the
  notification and unnamed actions numbered from 0, as in libnotify 0.8. Waits
  (`-w`) end on the timeout or Ctrl+C with libnotify's messages and exit codes, and
  take the notification down, so that no buttons outlive them.
- `-r` replaces a notification and shows the new one; `-p` prints the ID.
- Hints for progress bars (`value`), sounds (`suppress-sound`, `sound-name`),
  images (`image-path`), urgency and transient notifications.
- Backslash escapes in the body, as on Linux.
- Images from relative paths, absolute paths and `file://` URIs, including file
  names with spaces, `#`, `%` and non-ASCII characters. Only files on a drive of
  this computer are used: paths to other computers, mapped network drives and
  devices are refused before they are touched, so that text from elsewhere cannot
  make Windows sign in to another computer.
- Works on older versions of Windows 10 too, without the notification properties
  they lack.
- Version information and an icon embedded in `notify-send.exe`, which needs no
  Visual C++ runtime.
- Release builds for x86_64 and ARM64, tested before they are built, with checksums
  and build provenance attestations, and a PowerShell installer that sets
  notify-send up for the current user. It installs the ARM64 build on ARM64
  machines even from an x64 PowerShell, and can update while notify-send is waiting
  for a notification.

[Unreleased]: https://github.com/metinata/notify-send/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/metinata/notify-send/releases/tag/v0.1.0
