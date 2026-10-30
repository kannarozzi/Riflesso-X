# Development

## Requirements

On every system: [Rust](https://rustup.rs) stable and [Node.js](https://nodejs.org) 22 or
later. `crates/playfair` compiles C code, so a C compiler is needed too.

### Windows

- Windows 10 or 11, 64-bit.
- Rust with the **MSVC** toolchain and the
  [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
  (C++ workload).
- Optional: the *HEVC Video Extensions* from the Microsoft Store, to test H.265.

### Linux

Video goes through GStreamer, which picks the hardware decoder the system has (NVDEC on
NVIDIA, VA-API on Intel and AMD) and falls back to software. On Fedora:

```sh
sudo dnf group install c-development
sudo dnf install webkit2gtk4.1-devel openssl-devel libappindicator-gtk3-devel \
  librsvg2-devel libxdo-devel alsa-lib-devel \
  gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-bad-free
```

On Debian and Ubuntu:

```sh
sudo apt install build-essential libwebkit2gtk-4.1-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev libxdo-dev libasound2-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-bad gstreamer1.0-libav
```

Software decoding needs `gst-libav` (H.264 and H.265) or OpenH264. The log says which
decoder runs.

## Commands

All commands run from the repository root.

| Command | What it does |
|---|---|
| `npm install` | Installs the interface dependencies (first time). |
| `npm run tauri dev` | Runs the whole app with hot reload of the interface. |
| `npm run dev` | Only the interface, in a browser, with sample data (`ui/src/backend.ts`). |
| `npm run release` | Builds the installer, the portable and command-line zips and their checksums into `release/`. |
| `cargo run -p riflesso-cli -- --help` | Runs the command-line receiver (`riflesso.exe`). |
| `cargo test --workspace` | Runs the tests (one, marked `#[ignore]`, needs a GPU). |
| `cargo clippy --release --all-targets` | Lints; it must stay free of warnings. |

`npm run dev` first stops what an interrupted `tauri dev` of this folder left running
(`scripts/stop-leftovers.mjs`), which would otherwise keep ports 5173 and 7000 busy.

Windows Firewall asks for permission the first time the receiver starts: allow it on
private networks. On Linux, the receiver needs TCP 7000 and 7100, UDP for the streams
and UDP 5353 (mDNS); Fedora Workstation's default firewall zone already allows them,
and the announcement works alongside Avahi. Only one AirPlay receiver can use port 7000
at a time.

## Layout

```
apps/
  desktop/           the app (Tauri 2): tray, settings, receiver lifecycle; tauri.conf.json, icons
    src/setup/       setup mode: the app installs, updates and uninstalls itself (setup.html)
  cli/               command-line receiver (riflesso.exe, riflesso on Linux)
  installer/         the downloadable Windows installer: the app, compressed, started in setup mode
crates/
  airplay/           AirPlay protocol: discovery, handshake, streams, access control
  video/             decoding and drawing: Media Foundation and Direct3D 11 (win/),
                     GStreamer and OpenGL ES (linux/)
  audio/             playback with resampling (cpal)
  window/            the mirroring window, shared by the app and the command line
  updates/           new-version check and installer download (GitHub releases)
  playfair/          FairPlay key decryption (C code from playfair, GPL-3.0)
ui/                  interface: React 19, Tailwind CSS 4, TypeScript, Vite
  index.html         the app's window
  setup.html         the installer's window
  src/backend.ts     the only bridge to Rust; its types mirror apps/desktop/src/state.rs and settings.rs
  src/i18n.tsx       translations
scripts/             release build, dev helper, command-line install and uninstall scripts
docs/                protocol and development notes, Italian README, README images
```

The mirroring window is a native window (winit) drawn with Direct3D 11 or OpenGL on its
own thread (on Linux it is created for the first picture and destroyed when mirroring
stops, since Wayland cannot hide windows); the interface is a web view that is destroyed when the main window closes, so
only the receiver stays in memory while running in the notification area.

Settings, the receiver identity, the list of devices that entered the on-screen code
and the log (`riflesso-x.log`, release builds only) live in a `data` folder next to the
executable. The web view's profile is kept there too. On Linux, release builds use
`~/.local/share/riflesso-x` instead (`riflesso-x-cli` for the command line), since the
executable sits in a read-only place; debug builds keep `target/debug/data`.

The app icon's source is `apps/desktop/icons/app-icon.svg`; after changing it, run
`npm run tauri icon apps/desktop/icons/app-icon.svg` and delete the `android` and `ios`
folders it also creates.

## Debugging

| Option | Effect |
|---|---|
| `RUST_LOG=debug` | Detailed logs (`info,riflesso_airplay=debug` for the protocol only). |
| `--minimized` | Starts in the notification area only. |
| `--dump FILE` | Also saves the received video stream (Annex-B). |
| `--play FILE` | Replays a stream saved with `--dump`, without a sender. |
| `RIFLESSO_SOFTWARE_DECODE=1` | Forces software video decoding. |
| `RIFLESSO_FEATURES=0xLOW,0xHIGH` | Advertises exactly these feature bits. |

From Git Bash: `RUST_LOG=debug npm run tauri dev`.

## Installer

The downloadable `…-setup.exe` (`apps/installer`) is a small native program carrying
the app compressed. It unpacks the app to a temporary folder and starts it with
`--setup`: the app shows `setup.html` and copies itself to `%LOCALAPPDATA%\Riflesso X`,
adds the Start menu shortcut and the "Installed apps" entry. The installed app is its
own uninstaller (`"Riflesso X.exe" --uninstall`). Both accept `--silent`; the
installer also `--dir FOLDER` (the app goes into a "Riflesso X" folder inside it, which
is all that uninstalling removes, apart from files the user added there).

To work on the installer's interface, run `npm run dev` and open
`http://localhost:5173/setup.html?mode=install` (or `update`, `reinstall`, `uninstall`).
To run the real thing in development: `npm run tauri dev -- -- --setup`.

## Releases and signing

Pushing a tag such as `v1.0.0` runs [`.github/workflows/release.yml`](../.github/workflows/release.yml),
which builds everything with `scripts/release.mjs` and attaches the files to a draft
release. Signing with [SignPath](https://signpath.org) turns on by itself once the
repository has the secret `SIGNPATH_API_TOKEN` and the variable
`SIGNPATH_ORGANIZATION_ID` (optional: `SIGNPATH_PROJECT_SLUG`, default `riflesso-x`;
`SIGNPATH_POLICY_SLUG`, default `release-signing`). The SignPath project needs two
artifact configurations:

`app`
```xml
<?xml version="1.0" encoding="utf-8"?>
<artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
  <zip-file>
    <pe-file path="riflesso-x.exe" product-name="Riflesso X"><authenticode-sign /></pe-file>
    <pe-file path="riflesso.exe" product-name="Riflesso X"><authenticode-sign /></pe-file>
  </zip-file>
</artifact-configuration>
```

`installer`
```xml
<?xml version="1.0" encoding="utf-8"?>
<artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
  <zip-file>
    <pe-file path="riflesso-setup.exe" product-name="Riflesso X"><authenticode-sign /></pe-file>
  </zip-file>
</artifact-configuration>
```

## Command-line installer

`scripts/install.ps1` and `scripts/uninstall.ps1` install the command line from the
latest GitHub release into `%LOCALAPPDATA%\Riflesso X CLI` and manage the user's PATH.
To try the install script with a local build before publishing it:

```powershell
npm run release
$env:RIFLESSO_CLI_ZIP = "$PWD\release\Riflesso-X_1.0.0_x64-cli.zip"
Get-Content .\scripts\install.ps1 -Raw | Invoke-Expression
```

## Translations

`ui/src/i18n.tsx` holds every language; TypeScript checks that each one has all the
keys of the English reference. To add a language, add its dictionary and an entry in
`languages`, and its two tray menu labels in `tray_labels` (`apps/desktop/src/main.rs`).
