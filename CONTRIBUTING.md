# Contributing

Thanks for your interest in notify-send for Windows! Bug reports, ideas,
documentation fixes and pull requests are all welcome.

## Issues

- Search the [existing issues](https://github.com/metinata/notify-send/issues)
  first; someone may have reported the same thing.
- For bugs, use the bug report form. The output of `notify-send --version`, your
  Windows version and the exact command make a bug much easier to fix.
- notify-send aims to behave like libnotify's `notify-send` on Linux. Ideas that go
  beyond it are welcome, but please open an issue to discuss them before writing
  code.
- Security vulnerabilities should be reported privately as described in
  [SECURITY.md](SECURITY.md), not in public issues.

## Development setup

You need:

- Windows 10 or 11 (x64 or ARM64)
- [Rust](https://rustup.rs) 1.85 or newer
- The MSVC build tools: Visual Studio or Build Tools with the
  "Desktop development with C++" workload, which includes the Windows SDK

Then:

```console
> git clone https://github.com/metinata/notify-send
> cd notify-send
> cargo build
> cargo run -- "Hello" "from notify-send"
```

## Checks

Run these before you open a pull request:

```console
> cargo fmt --all
> cargo clippy --all-targets -- -D warnings
> cargo test
```

CI runs the same checks, builds for ARM64 and checks the minimum supported Rust
version (1.85).

`.cargo/config.toml` links the C runtime statically, so that `notify-send.exe`
runs without the Visual C++ Redistributable. A `RUSTFLAGS` environment variable
replaces that setting, and a test then fails; unset it to build.

### About the integration tests

The tests in `tests/cli.rs` run the real `notify-send.exe` and show real
notifications, as the app "notify-send tests". Most use low urgency, so they go
straight to the notification center; the tests of `--wait` pop up for a moment. The
tests remove what they showed. They also register "notify-send tests" as an app,
which then appears in Settings → System → Notifications. To remove it:

```powershell
Remove-Item HKCU:\Software\Classes\AppUserModelId\metinata.notify-send.62e94f17ec3dd098
```

That key belongs to "notify-send tests" alone; other app names you used keep
theirs.

Whether Windows can show notifications is checked once, by showing one directly
rather than through notify-send, so a broken notify-send fails the tests. Where
Windows cannot, such as in a service, the tests that need notifications are
skipped, with a warning that `cargo test` shows.

### Testing by hand

Clicks cannot be automated, so changes to buttons, actions or waiting need a test
by hand. With the build from `cargo build`:

```powershell
$ns = '.\target\debug\notify-send.exe'
# Click "Yes": prints yes. Run it again and click the notification: prints default.
& $ns -A default=Open -A yes=Yes -A no=No "Question" "Click a button"
# Close it with X: prints nothing, exit status 0.
& $ns -A yes=Yes "Question" "Close me"
# Click Dismiss: exits with 0. Run it again and close the console window: the
# notification goes away.
& $ns -u critical -w "Critical" "Stays until dismissed"
```

## Guidelines

- **Match Linux.** Options, output and exit codes should behave like libnotify's
  `notify-send` unless Windows makes that impossible. Document any difference in
  `README.md` and the reasoning in `docs/design.md`.
- **Never change an app's AUMID.** Windows keeps the user's notification settings
  per AUMID; the unit test `aumids_never_change` guards this.
- **Stay small.** notify-send is a single executable with one dependency,
  Microsoft's `windows` crate. Please discuss new dependencies in an issue first.
- **Explain `unsafe`.** Every `unsafe` block needs a `// SAFETY:` comment that says
  why it is sound. Clippy enforces this.
- **Test changes.** Unit tests live next to the code; end-to-end tests go in
  `tests/cli.rs`.
- **Record user-facing changes** in `CHANGELOG.md` under `[Unreleased]`.

Commit messages should start with a short summary in the imperative mood, for
example "Fix -r when the old notification is in the notification center", with
details in the body when the reason is not obvious.

## Releasing

For maintainers:

1. Set the new version in `Cargo.toml` and run `cargo check` to update `Cargo.lock`.
2. In `CHANGELOG.md`, rename `[Unreleased]` to `[X.Y.Z] - YYYY-MM-DD`, add a fresh
   empty `[Unreleased]` section above it, and update the links at the bottom.
3. Commit as "Release vX.Y.Z", then tag and push:

   ```console
   > git tag vX.Y.Z
   > git push origin main vX.Y.Z
   ```

The release workflow checks that the tag matches `Cargo.toml`, runs the tests,
builds x86_64 and ARM64 zips with checksums and provenance attestations, and
publishes a GitHub
release with the changelog section as its notes. The release also carries
`install.ps1`, which the installation command in the README downloads from the
latest release; keep the zip names it expects (`notify-send-<target>.zip`) stable.

4. Once the release is published, update the winget package. This opens a pull
   request at [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs)
   with [wingetcreate](https://github.com/microsoft/winget-create)
   (`winget install wingetcreate`), which asks you to sign in to GitHub:

   ```powershell
   $v = 'X.Y.Z'
   $base = "https://github.com/metinata/notify-send/releases/download/v$v"
   wingetcreate update metinata.notify-send --version $v --submit --urls `
     "$base/notify-send-x86_64-pc-windows-msvc.zip|x64" `
     "$base/notify-send-aarch64-pc-windows-msvc.zip|arm64"
   ```

   For the first release, create the package with `wingetcreate new` and the same
   two URLs instead. Use the package identifier `metinata.notify-send`, the moniker
   `notify-send`, and `notify-send.exe` as the portable command, as the README's
   installation command expects.

The winget package is updated by hand on purpose: automating it would mean
storing a token that can write to the repository in its secrets.

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE).
