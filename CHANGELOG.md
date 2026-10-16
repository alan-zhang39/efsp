# Changelog

All notable changes to efsp are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [1.0.0]

First release.

### Added

- **Match card before you accept.** About 1–2 seconds after a match is found,
  efsp shows the real ping, its own 1–5 bars and where the other end is. When
  the match ends it's saved to History.
- **Live graph during the match**: ping, jitter range and loss over the
  last minute, with the full graph kept for the match afterwards.
- **Servers page and region picker.** Your ping to
  every city the game checks in the background, measured passively without
  sending anything. Switch cities off to keep matchmaking away from them: efsp
  adds Windows Firewall rules (outbound UDP, `eFootball.exe` only) after a
  Windows permission prompt. Switch every city back on and Apply to remove
  them.
- **Match history**: rows or cards, search by server, filter by time range and
  server or peer, declined offers marked "Offer", a detail view with the full
  graph, and CSV export.
- **Settings**: dark, light or Windows theme; start with Windows and start in
  the tray; choose which figures the card and graph show; how long to keep
  history; delete all history.
- Tray icon, single instance, and a custom window frame.

### Good to know

- Windows 10 and 11 only. Needs [Npcap](https://npcap.com); efsp links to it on
  first start.
- The installer isn't signed yet, so Windows SmartScreen may say "Windows
  protected your PC": choose **More info → Run anyway**.
- efsp only measures. Apart from the region picker's firewall rules, it never
  touches the game or changes game traffic, and never shows or saves an
  opponent's address. It makes no requests of its own:
  the only packets it sends are pings to the match server, at most 5 a second.
- The region picker isn't endorsed by Konami and may make matchmaking slower.
  Its rules stay on when efsp is closed or uninstalled, until you switch the
  cities back on in efsp and Apply, or delete the `efsp: region picker` rules
  in Windows Defender Firewall.
  Peer-to-peer matches start through a game server, so switching off nearby
  cities may affect them too.
- IP geolocation by DB-IP (db-ip.com), CC BY 4.0.

[1.0.0]: https://github.com/alan-zhang39/efsp/releases/tag/v1.0.0
