# Contributing to efsp

Thanks for your interest! Bug reports, ideas and pull requests are all
welcome. For anything bigger than a small fix, please open an issue first.

## Ground rules

efsp runs next to an online game, so a few rules are not negotiable. Changes
that break them won't be merged.

- **Measure only.** Never delay, drop, inject or modify game packets.
- **Headers and timing only.** Never read, store or decode packet payloads.
- **Never touch the game**: no memory reads, no injection, no file changes, no
  process handles.
- **Opponent privacy.** Remote addresses live in memory only while in use.
  Never display, log, save or export an opponent's IP; show a country or
  region instead.
- **Gentle probing.** Everything efsp sends shares one budget of at most
  5 probes per second.
- **No network calls.** IP data ships with each release; nothing is looked up
  at runtime.
- **Firewall changes** go only through the elevated helper, only as rules named
  `efsp: region picker`, scoped to the eFootball executable.
- **Never commit** secrets, raw packet captures or real IP addresses. Traces
  must be anonymised (see [Traces](#traces)).

## Project layout

| Path            | What it is                                                                                                                                                    |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/core`   | Pure Rust logic: parsing, flows, classification, measurement, bars, history, firewall rules. No I/O, so it builds and tests on any OS.                        |
| `crates/helper` | `efsp-helper.exe`, the small elevated program that adds and removes efsp's firewall rules. It trusts nothing in its request.                                   |
| `src-tauri`     | The Tauri app: Npcap capture, port watching, ICMP probes, the monitor thread, SQLite history and the commands the UI calls.                                   |
| `src`           | The React + TypeScript UI (Tailwind, shadcn-style components on Base UI, uPlot graphs).                                                                       |
| `scripts`       | `update-ip-data.mjs`, which refreshes the cloud IP ranges (committed) and downloads the DB-IP databases (not committed).                                       |

## Setup

You need:

- [Node.js](https://nodejs.org) (LTS)
- [Rust](https://rustup.rs), stable, 1.88 or newer
- Tauri's system prerequisites for your OS:
  [tauri.app/start/prerequisites](https://tauri.app/start/prerequisites/)
  (on Windows: Microsoft C++ Build Tools and WebView2; on macOS: Xcode's
  command line tools)

Then, from the repository root:

```bash
npm ci
npm run build
```

`npm run build` matters even if you only work on Rust: Tauri compiles the
built UI (`dist/`) into the app, so `cargo` can't build `src-tauri` until it
exists.

### macOS or Linux: run the app from a recorded trace

Live capture is Windows-only. Elsewhere, replay an anonymised trace with the
developer-tools build. Use an absolute path: the app runs from `src-tauri/`,
so a relative one won't be found.

```bash
EFSP_REPLAY="$PWD/crates/core/tests/fixtures/traces/synthetic-lobby-then-match.jsonl" \
  npm run tauri dev -- --features dev-tools
```

`EFSP_REPLAY_SPEED=4` plays it four times faster. Replays can't be pinged, so
the card shows packet-based numbers only, and the region picker isn't
available.

### Windows: local builds

1. Install [Npcap](https://npcap.com) to capture live traffic.
2. Download the [Npcap SDK](https://npcap.com/#download) and unzip it. The
   `pcap` crate links against its `wpcap.lib`.
3. In PowerShell, from the repository root:

```powershell
$env:LIBPCAP_LIBDIR = "C:\path\to\npcap-sdk\Lib\x64"
$env:LIBPCAP_VER = "1.10.0"
npm run tauri dev -- --features dev-tools
```

`wpcap.dll` is delay-loaded, so the app still starts without Npcap installed
and shows the install screen.

Without the game running, you can replay a trace on Windows too:

```powershell
$env:EFSP_REPLAY = "$PWD\crates\core\tests\fixtures\traces\synthetic-lobby-then-match.jsonl"
npm run tauri dev -- --features dev-tools
```

Clear it afterwards (`Remove-Item Env:EFSP_REPLAY`), or every later run in that
window replays instead of capturing.

A plain dev build has no IP databases (only cloud servers get a city) and no
region-picker helper (the Servers page shows no on/off switches). To
include both, as the installer does:

```powershell
node scripts/update-ip-data.mjs --geo
cargo build --release -p efsp-helper
New-Item -ItemType Directory -Force src-tauri\binaries | Out-Null
Copy-Item target\release\efsp-helper.exe src-tauri\binaries\efsp-helper-x86_64-pc-windows-msvc.exe
npm run tauri dev -- --features dev-tools --config "$PWD\src-tauri\tauri.bundle.conf.json"
```

The databases (`src-tauri/resources/ipdb/`) and the helper copy
(`src-tauri/binaries/`) are gitignored.

`dev-tools` is a build feature that adds the trace recorder, the `EFSP_REPLAY`
override, sample history and a few measurement experiments. Release installers
are built without it.

## Checks

Run what CI runs before opening a pull request (after `npm run build`):

```bash
npm run typecheck && npm run lint && npm test
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

These work on Windows and macOS. On Linux, either install Tauri's Linux
prerequisites or add `--exclude efsp` to the `clippy` and `test` commands, as
CI's Linux job does; the app itself is checked on Windows.

Please add or update tests with your change. Pure logic belongs in
`crates/core`, where it can be tested without a network or Windows.

## Traces

- Recorded traces are header-only and anonymised before they touch disk: every
  remote IP becomes a reserved documentation address, and times are relative.
- Committed fixtures live in `crates/core/tests/fixtures/traces/`. A test
  rejects any trace containing a non-documentation address.

## Commits and pull requests

- Use [Conventional Commits](https://www.conventionalcommits.org/):
  `feat(core): …`, `fix(ui): …`, `docs: …`, `ci: …`.
- Keep pull requests focused on one change, and describe what changed and why.
- `cargo fmt` and Prettier (`npm run format`) handle formatting.

## Refreshing IP data

`node scripts/update-ip-data.mjs --ranges` refreshes the committed Google Cloud
and AWS range table (`crates/core/data/cloud-ranges.tsv`). If a refresh adds a
region, the `classify` tests fail until it's added to `place()` (or to
`UNPLACED` when its city isn't published).
