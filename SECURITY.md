# Security policy

## Supported versions

Only the latest release gets security fixes.

## Reporting a vulnerability

Please **don't open a public issue** for a security problem. Report it
privately through GitHub instead: on this repository's **Security** tab,
choose **Report a vulnerability**.

## What counts

efsp is designed so that it can't harm the game, your PC or other players.
Anything that breaks one of these promises is a security issue:

- **The elevated helper.** `efsp-helper.exe` runs with administrator rights
  after a Windows permission prompt. It should only ever add or remove
  firewall rules named `efsp: region picker`, blocking outbound UDP from
  `eFootball.exe` to the published IP ranges of the cities you chose. Getting
  it to do anything else is a vulnerability: other rules, other programs,
  arbitrary ranges, files or commands.
- **Privacy.** efsp never shows, logs, saves or exports an opponent's IP
  address. A way to recover one from the app, its history database, logs or
  exports is a vulnerability.
- **Game traffic.** efsp only reads packet headers and timing. A way to make it
  read packet contents, or delay, drop or change game traffic (other than the
  region picker rules you chose), is a vulnerability.
- **Network traffic.** efsp makes no requests of its own: no updates,
  telemetry or online lookups. The only packets it sends are pings to the
  current match server (or the last router before it), at most 5 a second. A
  way to make it contact anything else, or send more, is a vulnerability.
- **The app itself**: anything that lets web content or another program run
  code through efsp or call its commands.

Issues in Npcap, Windows, WebView2 or eFootball itself should go to their
vendors.

## Installers

Installers are built by this repository's GitHub Actions workflow and published
only on its [Releases page](https://github.com/alan-zhang39/efsp/releases).
They aren't code-signed yet, so Windows SmartScreen will warn before the first
run. Don't install efsp from anywhere else.
