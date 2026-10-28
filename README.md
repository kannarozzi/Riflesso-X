<p align="center">
  <img src="docs/images/icon.png" width="112" alt="Riflesso X icon">
</p>

<h1 align="center">Riflesso X</h1>

<p align="center">
  <b>Mirror your iPhone, iPad or Mac to a Windows PC over AirPlay.</b><br>
  Free and open source. Sharp up to 4K, instant to connect, light on your PC.
</p>

<p align="center">
  <a href="https://github.com/kannarozzi/Riflesso-X/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/kannarozzi/Riflesso-X?style=flat-square&color=8b7cf6"></a>
  <img alt="Windows 10 and 11" src="https://img.shields.io/badge/Windows-10%20%7C%2011-0e1218?style=flat-square">
  <a href="LICENSE.md"><img alt="License: GPL-3.0" src="https://img.shields.io/badge/license-GPL--3.0-5ad1e6?style=flat-square"></a>
  <img alt="Made with Rust and Tauri" src="https://img.shields.io/badge/made%20with-Rust%20%2B%20Tauri-f08bb8?style=flat-square">
  <a href="https://github.com/kannarozzi/Riflesso-X/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/kannarozzi/Riflesso-X?style=flat-square&color=f3c76a&logo=github"></a>
</p>

<p align="center">
  <a href="https://github.com/kannarozzi/Riflesso-X/releases/latest"><b>Download for Windows</b></a>
  &nbsp;·&nbsp;
  <a href="#getting-started">Getting started</a>
  &nbsp;·&nbsp;
  <a href="#support-riflesso-x">Support</a>
  &nbsp;·&nbsp;
  <a href="docs/README.it.md">Italiano</a>
</p>

<p align="center">
  <img src="docs/images/hero.png" width="820" alt="Riflesso X in light and dark theme, with an iPhone being mirrored">
</p>

## Why Riflesso X

- **Sharp picture.** Up to 4K with H.265, hardware decoding and GPU scaling, with
  NVIDIA RTX Video Super Resolution where available. Colours are shown the way the
  iPhone sends them (full-range BT.709).
- **Instant connection.** Tap *Screen Mirroring*, pick your PC, and the picture is
  there in a fraction of a second.
- **Sound included.** The device's audio plays on the PC, resampled to whatever
  your sound card wants.
- **Yours to control.** Let anyone on your network connect, or ask for a password or
  a code shown on the PC.
- **Light.** A 4.5 MB installer. Closing the window frees the interface completely
  and only the receiver stays, in the notification area, if you want it to.
- **Tidy.** Everything lives in one folder: settings, logs, even the web view's
  files. Uninstalling or deleting the folder leaves nothing behind.
- **Private.** No account, no telemetry, no cloud. The stream goes from your device
  to your PC over your local network and nowhere else. The only thing the app asks
  the internet is whether a newer version is out, and you can turn that off.
- **At home on Windows.** Follows the light or dark theme, speaks 10 languages, and
  the mirroring window sizes itself to fill the screen without hiding under the taskbar.

## Download

| | |
|---|---|
| **Installer** · `Riflesso-X_1.0.1_x64-setup.exe` | Installs for your user only, in a single folder you can choose, no administrator rights needed. |
| **Portable** · `Riflesso-X_1.0.1_x64-portable.zip` | Unzip anywhere and run `Riflesso X.exe`. To remove it, delete the folder. |
| **Command line** · `Riflesso-X_1.0.1_x64-cli.zip` | Just the `riflesso` command, without the app. Easiest installed with one line: see [Command line](#command-line). |

They are on the [releases page](https://github.com/kannarozzi/Riflesso-X/releases/latest),
with a `SHA256SUMS.txt` to check the downloads.

**Requirements:** Windows 10 or 11, 64-bit, with the Microsoft Edge WebView2 Runtime
(already part of Windows 11). For more than 1080p, install the free
[HEVC Video Extensions](https://apps.microsoft.com/detail/9n4wgh0z6vhq) from the
Microsoft Store.

> [!NOTE]
> Riflesso X uses free code signing provided by [SignPath.io](https://about.signpath.io),
> certificate by [SignPath Foundation](https://signpath.org): see the
> [code signing policy](CODE_SIGNING_POLICY.md). Until signed releases are out,
> Windows SmartScreen may warn you the first time: choose **More info → Run anyway**.
> You can verify the files against `SHA256SUMS.txt`, or build them from this source.

## Getting started

1. Start Riflesso X. The first time, Windows Firewall asks for permission: allow it
   on **private networks**.
2. Connect your iPhone, iPad or Mac to the **same Wi-Fi network** as the PC.
3. Open **Control Center → Screen Mirroring** and choose **Riflesso X**.

In the mirroring window, double-click or press <kbd>F11</kbd> for full screen and
<kbd>Esc</kbd> to leave it. Closing the window ends the mirroring (or only hides it,
if you prefer: see *Settings → App*).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/screens-dark.png">
  <img src="docs/images/screens-light.png" width="820" alt="Waiting for a device, the settings, and an AirPlay code shown on the PC">
</picture>

## Security

Choose who can mirror to your PC in *Settings → Receiver → Security*:

| Mode | What happens |
|---|---|
| **None** | Anyone on your network can connect. |
| **Password** | The device asks for a password once, then remembers it. |
| **On-screen code** | The PC shows a 4-digit code the first time a device connects; after that the device is remembered. |
| **Code every time** | A new code at every connection. |

The device presents the code as a password prompt. *Forget paired devices* makes
every device ask for a new code.

## Command line

Prefer a terminal? The `riflesso` command is the same receiver and mirroring
window, without the app: no installer, no interface. Install it from PowerShell:

```powershell
irm https://raw.githubusercontent.com/kannarozzi/Riflesso-X/main/scripts/install.ps1 | iex
```

It downloads the latest release, checks its SHA-256, installs it in
`%LOCALAPPDATA%\Riflesso X CLI` and adds it to your PATH. Then, in any terminal:

```sh
riflesso                              # receive as "Riflesso X"
riflesso -n "Living room" -r 1080p    # another name, at most 1080p
riflesso --code                       # show a code the first time a device connects
riflesso --password secret            # ask devices for a password
riflesso --hide-on-close              # closing the window doesn't disconnect
riflesso --help                       # all the options
```

Connections and codes are printed in the terminal; <kbd>Ctrl</kbd>+<kbd>C</kbd> quits.
Run the same line again to update. To remove it:

```powershell
irm https://raw.githubusercontent.com/kannarozzi/Riflesso-X/main/scripts/uninstall.ps1 | iex
```

## Troubleshooting

<details>
<summary><b>Riflesso X doesn't appear under Screen Mirroring</b></summary>

- Make sure the device and the PC are on the same network, and that the network is
  set as *Private* in Windows.
- Allow Riflesso X in Windows Firewall (*Allow an app through Windows Firewall*).
- VPNs and guest Wi-Fi networks often block device discovery.
- Another AirPlay receiver may be using port 7000: Riflesso X tells you so. Close the
  other receiver and press *Try again*.
</details>

<details>
<summary><b>The picture is limited to 1080p</b></summary>

iPhones only send more than 1080 lines with H.265. Install the
[HEVC Video Extensions](https://apps.microsoft.com/detail/9n4wgh0z6vhq) and make sure
*Use H.265* is on in *Settings*.
</details>

<details>
<summary><b>Where are my settings stored?</b></summary>

In the `data` folder next to `Riflesso X.exe` (for the installer,
`%LOCALAPPDATA%\Riflesso X\data`). Deleting it resets the app.
</details>

## Roadmap

- **Linux version**: planned. The video pipeline is Windows-specific today.
- Recording to MP4.
- Several devices at once.
- Starting with Windows.
- Audio-only AirPlay (music) and Google Cast.

Ideas and votes are welcome in the [issues](https://github.com/kannarozzi/Riflesso-X/issues).

## Building from source

```sh
npm install
npm run tauri dev     # run with hot reload
npm run release       # installer, portable zip and checksums in release/
```

You need Rust (MSVC toolchain), the Visual Studio C++ Build Tools and Node.js. See
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for details, and
[docs/PROTOCOL.md](docs/PROTOCOL.md) for how AirPlay mirroring works under the hood.

## Support Riflesso X

Riflesso X is free, open source and made in my spare time. If it's useful to you,
two things help a lot:

- **A star on GitHub**: it costs you a click and helps other people find the project.
- **A coffee on Ko-fi**: it keeps the work going, from new features to the Linux version.

<p align="center">
  <a href="https://github.com/kannarozzi/Riflesso-X/stargazers"><img alt="Star Riflesso X on GitHub" src="https://img.shields.io/github/stars/kannarozzi/Riflesso-X?style=for-the-badge&logo=github&label=Star&color=8b7cf6"></a>
  &nbsp;
  <a href="https://ko-fi.com/kannarozzi"><img alt="Support me on Ko-fi" src="https://img.shields.io/badge/Ko--fi-support%20me-f08bb8?style=for-the-badge&logo=kofi&logoColor=white"></a>
</p>

## Contributing

Bug reports, translations and pull requests are welcome: see
[CONTRIBUTING.md](.github/CONTRIBUTING.md).

## Acknowledgements

Riflesso X stands on the shoulders of the open-source AirPlay community:

- [UxPlay](https://github.com/FDH2/UxPlay) and [RPiPlay](https://github.com/FD-/RPiPlay):
  the protocol reference for screen mirroring.
- [playfair](https://github.com/EstebanKubata/playfair): FairPlay key decryption
  (included in `crates/playfair`).
- [shairplay](https://github.com/juhovh/shairplay): where much of this knowledge began.
- [rusty_aac](https://crates.io/crates/rusty_aac): AAC-ELD audio decoding in pure Rust.
- [Tauri](https://tauri.app), [React](https://react.dev) and
  [Tailwind CSS](https://tailwindcss.com) for the interface.

## License

Riflesso X is free software under the [GNU General Public License v3.0 or later](LICENSE.md).
You can use, study, change and share it; if you distribute a modified version, it
must stay under the same license.

AirPlay, iPhone, iPad and Mac are trademarks of Apple Inc. Riflesso X is an
independent project, not affiliated with or endorsed by Apple.
