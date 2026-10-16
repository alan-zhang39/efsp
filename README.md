# efsp
eFootball Server Picker (efsp) is a Windows app that aims to provide some QOL features to eFootball.

efsp is run alongside the PC version of eFootball.

[Download for Windows](https://github.com/alan-zhang39/efsp/releases/latest) ·
[How it works](#how-it-works) · [Is it safe?](#is-it-safe)

## Features

### Before you accept
As soon as a match is found, eFootball displays the connection in bars.
This is unreliable; for example, it's possible to have a 5 bar connection that feels extremely laggy, or a 4 bar connection that feels smooth.

Instead, efsp finds the match connection and measures it:

- **Ping** to the match
- **Where it is**: the server's city (e.g. "Frankfurt"), or the other player's
  country for a peer-to-peer match (e.g. "Canada (peer)")

<img width="2176" height="1456" alt="before-accept" src="https://github.com/user-attachments/assets/848fa65b-e06c-489c-a38d-553e74090651" />


### During the match

The card stays up for the whole match with a live graph of ping, jitter and
packet loss, so you can tell lag on your side from a bad connection. When the
match ends, efsp goes back to waiting, and the match and its full graph are
saved in History.

<img width="2176" height="1456" alt="during-match" src="https://github.com/user-attachments/assets/a56161c1-c0f3-4a26-92d8-28be721e1d73" />


### Pick your servers

The Servers page shows your ping to every city the game checks in the
background, measured without sending anything. Switch off the cities you don't
want to play on, and efsp blocks them with Windows Firewall rules (outbound UDP,
`eFootball.exe` only), after a Windows permission prompt.

- The rules stay on when efsp is closed or uninstalled. To undo, switch every
  city back on (the "All servers" switch) and Apply; that works any time, even
  mid-match. Or delete the rules named `efsp: region picker` in Windows
  Defender Firewall.
- p2p matches (e.g. friend matches) start through a game server, so switching
  off nearby cities may affect them too.

<img width="2176" height="1456" alt="servers" src="https://github.com/user-attachments/assets/fb802931-cdb0-4f6b-bbf1-58d01effb79c" />


### Match history

Matches are saved locally. Declined matches are saved as "offers".

<img width="2176" height="1456" alt="history" src="https://github.com/user-attachments/assets/7b5edce3-120d-4afe-a33b-90a6d629eccd" />


## Install

1. Download the installer from the
   [latest release](https://github.com/alan-zhang39/efsp/releases/latest).
2. The installer isn't signed, so Windows SmartScreen may say "Windows
   protected your PC". This is normal. Choose **More info → Run anyway**.
3. efsp needs [Npcap](https://npcap.com). If it's missing, efsp shows a link to install it.
4. Start eFootball and queue for a match.

Windows 10 and 11 only.

## Is it safe?

- **It only measures.** It reads the timing and headers of the game's network
  packets, never their contents, and never delays, drops or changes them,
  apart from the region picker's firewall rules for the cities you switch off.
- **It never touches the game.** No memory reading, no injection, no changes to
  game files. It finds the game's connections from Windows' own connection
  list, the same one `netstat` shows.
- **It sends very little.** To measure ping, efsp pings the match server with
  standard ICMP echo requests (what Windows' `ping` sends), at most 5 a second.
- **Your data stays on your PC.** No accounts, no telemetry, no online lookups:
  apart from those pings, efsp sends nothing. Opponents' IP addresses are never
  shown, saved or exported, only their country.
- **Open source.** You can read every line, and installers are built by this
  repository's public CI.

Note: efsp is not affiliated with or endorsed by Konami. Use at your own discretion.

## How it works

1. **Find the game's traffic.** efsp asks Windows which ports `eFootball.exe`
   has open and captures only those packets through Npcap, keeping at most the
   first 160 bytes of each and reading only the header fields.
2. **Find the match.** Packets are grouped into conversations. The match is the
   one carrying steady two-way traffic (about 100 packets a second), unlike
   the lobby's occasional requests.
3. **Measure it.** efsp pings the match server and works out ping, jitter and
   loss. If the server ignores pings, it estimates through the last router on
   the way, or falls back to packet timing alone.
4. **Say where it is.** The server's address is matched against Google Cloud's
   and AWS's published IP ranges to find its city. Other addresses are looked
   up in an offline database for their country. Nothing is looked up online.

The core logic is plain Rust with no I/O, tested by replaying recorded,
anonymised traffic. The app is built with [Tauri](https://tauri.app), and the
interface with React and TypeScript.

## Building from source

See [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: see [SECURITY.md](SECURITY.md).

## Credits and licence

- [MIT licence](LICENSE)
- IP geolocation by [DB-IP](https://db-ip.com), CC BY 4.0
- Packet capture by [Npcap](https://npcap.com)
- eFootball is a trademark of Konami Digital Entertainment. efsp is an
  independent project and uses no Konami assets.
