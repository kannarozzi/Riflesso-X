// Builds the files of a release into `release/`. On Windows:
//   Riflesso-X_<version>_x64-setup.exe     installer (apps/installer, per user, one folder)
//   Riflesso-X_<version>_x64-portable.zip  unzip and run; data stays beside it
//   Riflesso-X_<version>_x64-cli.zip       the command-line receiver (riflesso.exe)
// On Linux:
//   Riflesso-X_<version>_amd64.deb         package for Debian, Ubuntu and derivatives
//   Riflesso-X_<version>_x86_64.rpm        package for Fedora and derivatives
//   Riflesso-X_<version>_linux-x64-cli.tar.gz  the command-line receiver (riflesso)
// plus SHA256SUMS.txt, the checksums of the others. CI puts the files of both
// systems in one release, with one SHA256SUMS.txt for all.
//
// Usage: npm run release [app] [setup] [package]
//
// The stages let CI sign in between (.github/workflows/release.yml): the app
// is signed before the installer embeds it, then the installer is signed.
//   app      build the app (and on Linux its packages) and the command line
//   setup    build the installer around the app (Windows only)
//   package  collect the files into release/, with checksums
// Without arguments all three run.

import { execSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const config = JSON.parse(readFileSync(join(root, "apps", "desktop", "tauri.conf.json"), "utf8"));
const { version, productName } = config;
const name = productName.replaceAll(" ", "-");
const base = `${name}_${version}_x64`;
const linux = process.platform === "linux";
const out = join(root, "release");
const built = join(root, "target", "release");
// Windows' own tar writes standard zip paths (Compress-Archive uses
// backslashes); Git's tar, which may come first in PATH, cannot write zips.
const tar = join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe");

const run = (command) => execSync(command, { cwd: root, stdio: "inherit" });

/** Zips `files` (name in the zip → built file) into `<base>-<suffix>.zip`,
    inside a folder called `folder`, with the license and a README.txt. */
function zipFolder(suffix, folder, files, readme) {
  const staging = join(out, "staging");
  mkdirSync(join(staging, folder), { recursive: true });
  for (const [name, source] of Object.entries(files)) copyFileSync(source, join(staging, folder, name));
  copyFileSync(join(root, "LICENSE.md"), join(staging, folder, "LICENSE.md"));
  writeFileSync(join(staging, folder, "README.txt"), readme.join("\r\n"));
  const zip = join(out, `${base}-${suffix}.zip`);
  execSync(`"${tar}" -a -c -f "${zip}" "${folder}"`, { cwd: staging, stdio: "inherit" });
  rmSync(staging, { recursive: true, force: true });
  if (!existsSync(zip)) throw new Error(`${zip} was not created`);
}

const stages = process.argv.slice(2);
const wants = (stage) => stages.length === 0 || stages.includes(stage);

if (wants("app")) {
  // On Linux this also builds the .deb and .rpm (apps/desktop/tauri.linux.conf.json).
  run("npm run tauri build");
  run("cargo build --release -p riflesso-cli");
}
// After the app: the installer embeds it (apps/installer/build.rs).
if (wants("setup") && !linux) run("cargo build --release -p riflesso-setup");
if (wants("package")) {
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out);
  if (linux) packageLinux();
  else packageWindows();
  writeChecksums();
}

function packageWindows() {
  copyFileSync(join(built, "riflesso-setup.exe"), join(out, `${base}-setup.exe`));

  zipFolder("portable", productName, { [`${productName}.exe`]: join(built, "riflesso-x.exe") }, [
    `${productName} ${version} (portable)`,
    "",
    `Run "${productName}.exe". Settings and logs are kept in the "data" folder`,
    "created next to it: to remove the app, delete this folder.",
    "",
    "Requires Windows 10 or 11 (64-bit) with the Microsoft Edge WebView2 Runtime,",
    "which Windows 11 already includes.",
    "",
    "https://github.com/kannarozzi/Riflesso-X",
    "",
  ]);

  zipFolder("cli", `${productName} CLI`, { "riflesso.exe": join(built, "riflesso.exe") }, [
    `${productName} ${version}, command line`,
    "",
    "The same AirPlay receiver and mirroring window as the app, without its",
    "interface. The easiest way to install it, with the riflesso command on",
    "your PATH, is one line in PowerShell:",
    "",
    "  irm https://raw.githubusercontent.com/kannarozzi/Riflesso-X/main/scripts/install.ps1 | iex",
    "",
    "Or use it from this folder: open a terminal here and run, for example:",
    "",
    '  riflesso                         receive as "Riflesso X"',
    '  riflesso -n "Living room"        another name',
    "  riflesso --code                  show a code the first time a device connects",
    "  riflesso --password secret       ask devices for a password",
    "  riflesso --help                  all the options",
    "",
    'Press Ctrl+C to quit. Data is kept in the "data" folder created next to',
    "riflesso.exe: to remove the program, delete this folder.",
    "",
    "https://github.com/kannarozzi/Riflesso-X",
    "",
  ]);
}

/** The packages Tauri built (named after the product, with a space), and the
    command line as a .tar.gz, which keeps its executable bit. */
function packageLinux() {
  const bundle = join(built, "bundle");
  const only = (folder, extension) => {
    const found = readdirSync(join(bundle, folder)).filter((file) => file.includes(version) && file.endsWith(extension));
    if (found.length !== 1) throw new Error(`expected one ${extension} for ${version} in ${folder}, found ${found.length}`);
    return join(bundle, folder, found[0]);
  };
  copyFileSync(only("deb", ".deb"), join(out, `${name}_${version}_amd64.deb`));
  copyFileSync(only("rpm", ".rpm"), join(out, `${name}_${version}_x86_64.rpm`));

  const staging = join(out, "staging");
  const folder = "riflesso-x-cli";
  mkdirSync(join(staging, folder), { recursive: true });
  copyFileSync(join(built, "riflesso"), join(staging, folder, "riflesso"));
  chmodSync(join(staging, folder, "riflesso"), 0o755);
  copyFileSync(join(root, "LICENSE.md"), join(staging, folder, "LICENSE.md"));
  writeFileSync(
    join(staging, folder, "README.txt"),
    [
      `${productName} ${version}, command line`,
      "",
      "The same AirPlay receiver and mirroring window as the app, without its",
      "interface. The easiest way to install it, with the riflesso command in",
      "~/.local/bin, is one line in a terminal:",
      "",
      "  curl -fsSL https://raw.githubusercontent.com/kannarozzi/Riflesso-X/main/scripts/install.sh | sh",
      "",
      "Or run it from this folder, for example:",
      "",
      '  ./riflesso                       receive as "Riflesso X"',
      '  ./riflesso -n "Living room"      another name',
      "  ./riflesso --code                show a code the first time a device connects",
      "  ./riflesso --help                all the options",
      "",
      "It needs GStreamer with its base and bad plugins (and gst-libav or OpenH264",
      "for software decoding): see install.sh for each distribution's packages.",
      "Data is kept in ~/.local/share/riflesso-x-cli.",
      "",
      "https://github.com/kannarozzi/Riflesso-X",
      "",
    ].join("\n"),
  );
  const archive = join(out, `${name}_${version}_linux-x64-cli.tar.gz`);
  run(`tar --owner=0 --group=0 --numeric-owner -czf "${archive}" -C "${staging}" ${folder}`);
  rmSync(staging, { recursive: true, force: true });
}

function writeChecksums() {
  // Checksums, in the format `sha256sum -c` reads.
  const sums = readdirSync(out)
    .filter((name) => name !== "SHA256SUMS.txt")
    .map((name) => `${createHash("sha256").update(readFileSync(join(out, name))).digest("hex")}  ${name}`);
  writeFileSync(join(out, "SHA256SUMS.txt"), sums.join("\n") + "\n");

  console.log(`\nRelease files in ${out}:`);
  for (const name of readdirSync(out)) console.log(`  ${name}`);
}
