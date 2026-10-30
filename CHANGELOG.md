# Changelog

## 1.1.0

- **Linux.** Riflesso X now runs on Linux too, as a .deb package (Ubuntu,
  Debian, Mint, Pop!_OS) and an .rpm package (Fedora), plus the `riflesso`
  command line, installable with one line (`install.sh`). Video is decoded by
  GStreamer, in hardware where the GPU allows (NVIDIA NVDEC, VA-API on Intel
  and AMD), and drawn with OpenGL; Wayland and X11 are both supported.
- On Linux, settings and the receiver identity are kept in
  `~/.local/share/riflesso-x`.

## 1.0.1

- Receiver settings (name, video quality, codec, security) now apply as soon
  as they are chosen: there is no Save button to miss at the bottom of the
  page any more ([#2](https://github.com/kannarozzi/Riflesso-X/issues/2)).
  The name and the password apply when you leave the field or press Enter.
- If devices are connected, Riflesso X asks before applying a change, since
  restarting the receiver disconnects them.
- Fixed: two changes in quick succession could leave two receivers running.

## 1.0.0

First public release.

- AirPlay screen mirroring from iPhone, iPad and Mac to Windows 10 and 11.
- H.264 and H.265 up to 4K, hardware decoding, Direct3D 11 presentation with
  NVIDIA RTX Video Super Resolution where available.
- Audio (AAC-ELD) with resampling for any sound card.
- Security: none, password, on-screen code (first time only) or code every time.
- Mirroring window that fits the screen's work area; full screen with F11 or a
  double click.
- Choice of what closing each window does: quit or keep receiving from the
  notification area; disconnect the device or only hide the picture.
- Interface in 10 languages, light and dark theme following Windows.
- An installer in the app's own style (4 MB, for the current user, no
  administrator rights, in a folder of your choice), with an uninstaller to
  match, and a portable zip; both keep all data in the app's own folder.
- New-version notices, with "Update now", "Remind me later" and "Skip this
  version"; the check can be turned off.
- Command-line receiver, `riflesso`, installable on its own with one PowerShell
  line (`install.ps1`): the same mirroring window, with options for name,
  resolution, security and closing behaviour.
