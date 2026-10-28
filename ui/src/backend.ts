// Bridge to the Rust side (apps/desktop). Types mirror apps/desktop/src/state.rs
// and settings.rs: keep them in sync.

import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type Resolution = "auto" | "1080p" | "1440p" | "2160p";

export type CloseAction = "quit" | "tray";
export type MirrorCloseAction = "disconnect" | "hide";
export type SecurityMode = "none" | "password" | "code" | "codeEveryTime";

/** Shortest accepted password (`MIN_PASSWORD_LEN` in settings.rs). */
export const MIN_PASSWORD_LEN = 4;

export interface Settings {
  name: string;
  resolution: Resolution;
  hevc: boolean;
  /** A code from `languages` in i18n.tsx. */
  language: string;
  closeAction: CloseAction;
  mirrorCloseAction: MirrorCloseAction;
  security: SecurityMode;
  /** Used when `security` is "password". */
  password: string;
  /** Ask GitHub once a day whether a newer version is out. */
  checkUpdates: boolean;
  /** A version the user chose to skip. */
  skippedVersion: string;
  /** No update notice before this time (Unix seconds). */
  remindAfter: number;
}

/** A newer version to tell the user about. */
export interface UpdateInfo {
  version: string;
  /** Release notes, as written on GitHub (Markdown). */
  notes: string;
  page: string;
  /** The installed copy can update itself; a portable one gets the download page. */
  installable: boolean;
}

export type Status =
  | { state: "starting" }
  | { state: "running" }
  | { state: "error"; message: string }
  | { state: "playback" };

export interface Device {
  id: string;
  name: string;
  model: string;
  kind: "iphone" | "ipad" | "mac" | "other";
  mirroring: boolean;
  video: { codec: string; width: number; height: number } | null;
}

export interface Snapshot {
  version: string;
  status: Status;
  settings: Settings;
  hevcAvailable: boolean;
  requested: { width: number; height: number; hevc: boolean };
  devices: Device[];
  /** Code to show while a device asks for it. */
  code: string | null;
  update: UpdateInfo | null;
}

/** Live state of the app, updated whenever the backend emits a change. */
export function useSnapshot(): Snapshot | null {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);

  useEffect(() => {
    if (!isTauri()) {
      setSnapshot(previewSnapshot);
      previewListener = setSnapshot;
      return;
    }
    let active = true;
    const unlisten = listen<Snapshot>("snapshot", (event) => setSnapshot(event.payload));
    invoke<Snapshot>("get_snapshot").then((s) => active && setSnapshot(s));
    return () => {
      active = false;
      unlisten.then((stop) => stop());
    };
  }, []);

  return snapshot;
}

/** Saves the settings; the receiver restarts if name, resolution or codec
    changed. Rejects with a message. */
export async function saveSettings(settings: Settings): Promise<void> {
  if (!isTauri()) {
    await new Promise((resolve) => setTimeout(resolve, 600));
    previewSnapshot.settings = settings;
    previewListener?.({ ...previewSnapshot });
    return;
  }
  await invoke("save_settings", { settings });
}

/** Starts the receiver again (after an error such as a busy port). */
export async function restartReceiver(): Promise<void> {
  if (isTauri()) await invoke("restart_receiver");
}

/** Forgets the devices paired with an on-screen code. */
export async function forgetPairedDevices(): Promise<void> {
  if (isTauri()) await invoke("forget_paired_devices");
}

/** Asks GitHub now; resolves to whether a newer version exists. */
export async function checkForUpdates(): Promise<boolean> {
  if (isTauri()) return invoke("check_for_updates");
  await new Promise((resolve) => setTimeout(resolve, 600));
  return false;
}

/** Downloads and starts the installer (the app then quits), or opens the
    download page for a portable copy. */
export async function installUpdate(): Promise<void> {
  if (isTauri()) return invoke("install_update");
  await new Promise((resolve) => setTimeout(resolve, 1500));
}

export async function remindUpdateLater(): Promise<void> {
  if (isTauri()) return invoke("remind_update_later");
  previewSnapshot.update = null;
  previewListener?.({ ...previewSnapshot });
}

export async function skipUpdate(): Promise<void> {
  if (isTauri()) return invoke("skip_update");
  previewSnapshot.update = null;
  previewListener?.({ ...previewSnapshot });
}

/** Opens the project's GitHub page in the default browser. */
export async function openRepository(): Promise<void> {
  if (isTauri()) await invoke("open_repository");
  else window.open("https://github.com/kannarozzi/Riflesso-X", "_blank");
}

/** Brings the mirroring window to the front. */
export async function showMirror(): Promise<void> {
  if (isTauri()) await invoke("show_mirror");
}

/** The main window has no system title bar: these back its own buttons. */
export async function minimizeWindow(): Promise<void> {
  if (isTauri()) await getCurrentWindow().minimize();
}

/** Closing destroys the web view; the app keeps running in the tray. */
export async function closeWindow(): Promise<void> {
  if (isTauri()) await getCurrentWindow().close();
}

// Sample data for working on the UI in a plain browser (`npm run dev`).
let previewListener: ((snapshot: Snapshot) => void) | null = null;

const previewSnapshot: Snapshot = {
  version: "1.0.0",
  status: { state: "running" },
  settings: {
    name: "Riflesso X",
    resolution: "auto",
    hevc: true,
    language: "en",
    closeAction: "quit",
    mirrorCloseAction: "disconnect",
    security: "none",
    password: "",
    checkUpdates: true,
    skippedVersion: "",
    remindAfter: 0,
  },
  hevcAvailable: true,
  requested: { width: 2560, height: 1440, hevc: true },
  devices: [
    {
      id: "192.168.1.20:52346",
      name: "iPhone",
      model: "iPhone19,2",
      kind: "iphone",
      mirroring: true,
      video: { codec: "H.265", width: 664, height: 1440 },
    },
  ],
  code: null,
  update: null,
};
