# Security policy

## Supported versions

Security fixes are released for the latest version only.

## Reporting a vulnerability

Please do not report security vulnerabilities in public issues.

Report them privately through GitHub instead: open the repository's **Security**
tab and choose **Report a vulnerability**
([direct link](https://github.com/metinata/notify-send/security/advisories/new)).

Please include what you found, the steps to reproduce it, and the impact you expect.
You will get a reply as soon as possible and will be kept informed while a fix is
prepared. The published advisory will credit you, unless you prefer otherwise.

Examples of security issues in notify-send:

- text, action names or file names that change the structure of the notification,
  for example to add buttons, links or activations that were not asked for
- showing an image from a different location than the one given
- making notify-send or Windows connect to another computer, for example through
  an image path
- writing to the registry or to files outside notify-send's own: the
  `HKEY_CURRENT_USER\Software\Classes\AppUserModelId\metinata.notify-send*` keys
  and the `%LOCALAPPDATA%\notify-send` folder
- the install script installing anything other than the release's files
