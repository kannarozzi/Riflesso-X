// Stops what an interrupted `npm run tauri dev` of this project leaves behind:
// Vite still holding port 5173 and the debug build holding port 7000, which
// make the next run fail ("Port 5173 is already in use"). Only processes
// started from this folder are touched. `--dry-run` only lists them.
//
// `tauri dev` starts this script and the new debug build at the same time, so
// the app is stopped only if it has been running for a while: then it is a
// leftover, not the one this run just launched.

import { execFileSync } from "node:child_process";
import { resolve } from "node:path";

if (process.platform !== "win32") process.exit(0);

const dryRun = process.argv.includes("--dry-run");
const root = resolve(import.meta.dirname, "..").toLowerCase();
const VITE = "\\vite\\bin\\vite.js";
const APP = "\\target\\debug\\riflesso-x.exe";
const APP_MIN_AGE_MS = 15_000;

const list = `Get-CimInstance Win32_Process -Filter "Name='node.exe' or Name='riflesso-x.exe'" | ForEach-Object { "$($_.ProcessId)\`t$($_.CreationDate.ToString('o'))\`t$($_.CommandLine)" }`;
const output = execFileSync("powershell", ["-NoProfile", "-Command", list], { encoding: "utf8" });

for (const line of output.split(/\r?\n/)) {
  const [pid, created = "", commandLine = ""] = line.split("\t");
  const command = commandLine.toLowerCase();
  if (!command.includes(root)) continue;
  const age = Date.now() - Date.parse(created);
  const leftover = command.includes(VITE) || (command.includes(APP) && age > APP_MIN_AGE_MS);
  if (!leftover) continue;
  if (dryRun) {
    console.log(`would stop ${pid} (running for ${Math.round(age / 1000)} s): ${commandLine}`);
    continue;
  }
  try {
    process.kill(Number(pid));
    console.log(`stopped leftover process ${pid}`);
  } catch {
    // Already gone.
  }
}
