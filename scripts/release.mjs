// Builds the files of a release into `release/`:
//   Riflesso-X_<version>_x64-setup.exe     installer (apps/installer, per user, one folder)
//   Riflesso-X_<version>_x64-portable.zip  unzip and run; data stays beside it
//   Riflesso-X_<version>_x64-cli.zip       the command-line receiver (riflesso.exe)
//   SHA256SUMS.txt                         checksums of the three
//
// Usage: npm run release [app] [setup] [package]
//
// The stages let CI sign in between (.github/workflows/release.yml): the app
// is signed before the installer embeds it, then the installer is signed.
//   app      build the app and the command line into target/release
//   setup    build the installer around the app
//   package  collect the files into release/, with checksums
// Without arguments all three run.

import { execSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const config = JSON.parse(readFileSync(join(root, "apps", "desktop", "tauri.conf.json"), "utf8"));
const { version, productName } = config;
const base = `${productName.replaceAll(" ", "-")}_${version}_x64`;
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
  run("npm run tauri build");
  run("cargo build --release -p riflesso-cli");
}
// After the app: the installer embeds it (apps/installer/build.rs).
if (wants("setup")) run("cargo build --release -p riflesso-setup");
if (wants("package")) packageRelease();

function packageRelease() {
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out);

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

  // Checksums, in the format `sha256sum -c` reads.
  const sums = readdirSync(out)
    .filter((name) => name !== "SHA256SUMS.txt")
    .map((name) => `${createHash("sha256").update(readFileSync(join(out, name))).digest("hex")}  ${name}`);
  writeFileSync(join(out, "SHA256SUMS.txt"), sums.join("\n") + "\n");

  console.log(`\nRelease files in ${out}:`);
  for (const name of readdirSync(out)) console.log(`  ${name}`);
}
