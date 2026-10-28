// Bridge to the installer's Rust side (apps/desktop/src/setup/mod.rs). In a plain
// browser (`npm run dev`, then /setup.html?mode=update) it simulates one.

import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type SetupMode = "install" | "update" | "reinstall" | "uninstall";

export interface SetupInfo {
  mode: SetupMode;
  version: string;
  installedVersion: string | null;
  installDir: string;
  /** Only a new installation can choose its folder; updates stay put. */
  canChooseDir: boolean;
  appRunning: boolean;
}

export interface ChosenFolder {
  /** The folder the app will go into (a "Riflesso X" folder in the chosen one). */
  dir: string;
  /** False if it needs administrator rights. */
  writable: boolean;
}

export async function setupInfo(): Promise<SetupInfo> {
  if (isTauri()) return invoke("setup_info");
  const mode = (new URLSearchParams(location.search).get("mode") ?? "install") as SetupMode;
  return {
    mode,
    version: "1.0.0",
    installedVersion: mode === "install" ? null : mode === "update" ? "0.9.0" : "1.0.0",
    installDir: "C:\\Users\\You\\AppData\\Local\\Riflesso X",
    canChooseDir: mode === "install",
    appRunning: mode === "update",
  };
}

/** Steps reported while working, with how far along they are (0–1). */
export function onProgress(callback: (step: string, fraction: number) => void): () => void {
  if (!isTauri()) {
    previewProgress = callback;
    return () => (previewProgress = null);
  }
  const unlisten = listen<[string, number]>("progress", (event) => callback(...event.payload));
  return () => void unlisten.then((stop) => stop());
}

/** Asks for a folder with the system dialog; null if cancelled. */
export async function chooseFolder(current: string): Promise<ChosenFolder | null> {
  if (isTauri()) return invoke("choose_folder", { current });
  return { dir: "C:\\Program Files\\Riflesso X", writable: false };
}

export async function install(dir: string, desktopShortcut: boolean): Promise<void> {
  if (isTauri()) return invoke("install", { dir, desktopShortcut });
  return simulate(["closing", "copying", "shortcuts", "registering"]);
}

export async function uninstall(): Promise<void> {
  if (isTauri()) return invoke("uninstall_app");
  return simulate(["closing", "removing"]);
}

/** Starts Riflesso X and closes the setup. */
export async function launchApp(): Promise<void> {
  if (isTauri()) await invoke("launch_app");
}

export async function openLicense(): Promise<void> {
  if (isTauri()) await invoke("open_license");
}

export async function closeWindow(): Promise<void> {
  if (isTauri()) await getCurrentWindow().close();
}

export async function minimizeWindow(): Promise<void> {
  if (isTauri()) await getCurrentWindow().minimize();
}

let previewProgress: ((step: string, fraction: number) => void) | null = null;

async function simulate(steps: string[]) {
  for (const [i, step] of steps.entries()) {
    previewProgress?.(step, (i + 1) / (steps.length + 1));
    await new Promise((resolve) => setTimeout(resolve, 450));
  }
}
