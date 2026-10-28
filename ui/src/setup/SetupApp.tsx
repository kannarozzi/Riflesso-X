import { useEffect, useState, type ReactNode } from "react";
import { Minus, X } from "lucide-react";
import { useT } from "../i18n";
import icon from "./icon.png";
import {
  chooseFolder,
  closeWindow,
  install,
  launchApp,
  minimizeWindow,
  onProgress,
  openLicense,
  setupInfo,
  uninstall,
  type SetupInfo,
} from "./bridge";

type Phase = "ready" | "working" | "done" | "error";

/** The installer: the app icon standing on the horizon of the "Specchio"
    identity, whose iridescent line fills up as the work progresses. */
export function SetupApp() {
  const t = useT();
  const [info, setInfo] = useState<SetupInfo | null>(null);
  const [phase, setPhase] = useState<Phase>("ready");
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState("");
  const [desktopShortcut, setDesktopShortcut] = useState(false);
  const [dir, setDir] = useState("");
  const [writable, setWritable] = useState(true);

  useEffect(() => {
    setupInfo().then((info) => {
      setInfo(info);
      setDir(info.installDir);
    });
    return onProgress((_, fraction) => setProgress(fraction));
  }, []);

  if (!info) return null;
  const removing = info.mode === "uninstall";

  async function run() {
    setPhase("working");
    setProgress(0.05);
    try {
      await (removing ? uninstall() : install(dir, desktopShortcut));
      setProgress(1);
      setPhase("done");
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  }

  return (
    <div className="flex h-full flex-col">
      <TitleBar />

      <main data-tauri-drag-region className="flex flex-1 flex-col items-center px-10">
        <Stage progress={phase === "ready" ? 0 : progress} lit={phase === "done" && !removing} faded={phase === "done" && removing} />

        <p data-tauri-drag-region className="mt-1 font-display text-[15px] leading-none tracking-[0.12em]">
          RIFLESSO X <span className="font-mono text-[11px] tracking-normal text-faint">{info.version}</span>
        </p>

        <div className="mt-4 flex min-h-[3.5rem] flex-col items-center text-center">
          <Message info={info} phase={phase} error={error} />
        </div>

        <div className="mt-2 flex items-center gap-2.5">
          <Actions
            info={info}
            phase={phase}
            blocked={!writable}
            onRun={run}
            onRetry={() => {
              setPhase("ready");
              setProgress(0);
            }}
          />
        </div>

        {phase === "ready" && info.canChooseDir && (
          <div className="mt-4 w-full max-w-md">
            <div className="flex items-center gap-2">
              <span
                title={dir}
                className={`min-w-0 flex-1 truncate rounded-lg bg-glass px-3 py-1.5 font-mono text-[11px] text-ink-soft ${
                  writable ? "shadow-[0_0_0_1px_var(--color-edge)]" : "shadow-[0_0_0_1px_var(--color-danger)]"
                }`}
              >
                {dir}
              </span>
              <button
                type="button"
                onClick={async () => {
                  const chosen = await chooseFolder(dir);
                  if (chosen) {
                    setDir(chosen.dir);
                    setWritable(chosen.writable);
                  }
                }}
                className="shrink-0 rounded-lg px-2.5 py-1.5 text-xs font-semibold text-ink-soft transition hover:bg-edge hover:text-ink"
              >
                {t("setup.change")}
              </button>
            </div>
            {!writable && <p className="mt-1.5 text-center text-[11px] text-danger">{t("setup.notWritable")}</p>}
          </div>
        )}

        {phase === "ready" && info.canChooseDir && (
          <label className="mt-3 flex cursor-pointer items-center gap-2 text-xs text-dim">
            <input
              id="desktop-shortcut"
              type="checkbox"
              checked={desktopShortcut}
              onChange={(e) => setDesktopShortcut(e.target.checked)}
              className="size-3.5 accent-[var(--color-ink)]"
            />
            {t("setup.desktopShortcut")}
          </label>
        )}
      </main>

      <footer className="flex shrink-0 items-center justify-between gap-4 px-5 pb-3 text-[11px] text-faint">
        <span className="truncate">{!removing && t("setup.userOnly")}</span>
        <button type="button" onClick={openLicense} className="shrink-0 hover:text-ink hover:underline">
          {t("setup.license")} · GPL-3.0
        </button>
      </footer>
    </div>
  );
}

function Message({ info, phase, error }: { info: SetupInfo; phase: Phase; error: string }) {
  const t = useT();
  const removing = info.mode === "uninstall";
  const title = (text: string) => <h1 className="text-[19px] font-semibold tracking-tight">{text}</h1>;
  const hint = (text: string) => <p className="mt-1.5 max-w-sm text-[13px] text-dim">{text}</p>;

  switch (phase) {
    case "working":
      return title(t(removing ? "setup.removing" : info.mode === "update" ? "setup.updating" : "setup.installing"));
    case "done":
      return (
        <>
          {title(t(removing ? "setup.removed" : "setup.done"))}
          {hint(t(removing ? "setup.removedHint" : "setup.doneHint"))}
        </>
      );
    case "error":
      return (
        <>
          {title(t("setup.error"))}
          <p className="mt-1.5 max-w-sm text-[12px] break-words text-danger">{error}</p>
        </>
      );
    case "ready": {
      const running = info.appRunning ? ` ${t("setup.willClose")}` : "";
      if (removing) return <>{title(t("setup.uninstallTitle"))}{hint(t("setup.uninstallHint") + running)}</>;
      if (info.mode === "update")
        return hint(t("setup.updateFrom", { from: info.installedVersion ?? "", to: info.version }) + running);
      if (info.mode === "reinstall") return hint(t("setup.reinstallHint", { version: info.version }) + running);
      return hint(t("setup.tagline") + running);
    }
  }
}

function Actions({
  info,
  phase,
  blocked,
  onRun,
  onRetry,
}: {
  info: SetupInfo;
  phase: Phase;
  /** The chosen folder cannot be written. */
  blocked: boolean;
  onRun: () => void;
  onRetry: () => void;
}) {
  const t = useT();
  const removing = info.mode === "uninstall";
  switch (phase) {
    case "working":
      return null;
    case "done":
      return removing ? (
        <Button onClick={closeWindow}>{t("setup.close")}</Button>
      ) : (
        <>
          <Button onClick={launchApp}>{t("setup.open")}</Button>
          <Button secondary onClick={closeWindow}>
            {t("setup.close")}
          </Button>
        </>
      );
    case "error":
      return (
        <>
          <Button onClick={onRetry}>{t("setup.retry")}</Button>
          <Button secondary onClick={closeWindow}>
            {t("setup.close")}
          </Button>
        </>
      );
    case "ready":
      return removing ? (
        <>
          <Button danger onClick={onRun}>
            {t("setup.remove")}
          </Button>
          <Button secondary onClick={closeWindow}>
            {t("setup.cancel")}
          </Button>
        </>
      ) : (
        <Button onClick={onRun} disabled={blocked}>
          {t(info.mode === "update" ? "setup.update" : info.mode === "reinstall" ? "setup.reinstall" : "setup.install")}
        </Button>
      );
  }
}

/** The app icon on the horizon line (the progress bar) with its reflection. */
function Stage({ progress, lit, faded }: { progress: number; lit: boolean; faded: boolean }) {
  const image = (
    <img
      src={icon}
      alt=""
      draggable={false}
      className="size-[84px] select-none"
      // A hairline and a soft shadow keep the dark icon visible on the dark theme.
      style={{ filter: "drop-shadow(0 0 0.6px var(--color-edge)) drop-shadow(0 10px 16px rgb(0 0 0 / 0.22))" }}
    />
  );
  return (
    <div
      data-tauri-drag-region
      className={`mt-2 flex flex-col items-center transition-opacity duration-700 ${faded ? "opacity-30" : ""}`}
    >
      <div data-tauri-drag-region className={`transition-transform duration-500 ${lit ? "scale-105" : ""}`}>
        {image}
      </div>
      <div className="relative mt-1 h-[2px] w-72 overflow-hidden rounded-full bg-edge">
        <div
          className="bg-iris absolute inset-y-0 left-0 rounded-full transition-[width] duration-500 ease-out"
          style={{ width: `${Math.round(progress * 100)}%` }}
        />
        {lit && (
          <span
            className="absolute inset-y-0 left-0 w-1/3 bg-linear-to-r from-transparent via-white to-transparent"
            style={{ animation: "sheen 1.2s ease-out 0.3s both" }}
          />
        )}
      </div>
      <div className="reflection flex h-9 items-end overflow-hidden opacity-20" aria-hidden>
        {image}
      </div>
    </div>
  );
}

function Button({
  onClick,
  secondary,
  danger,
  disabled,
  children,
}: {
  onClick: () => void;
  secondary?: boolean;
  danger?: boolean;
  disabled?: boolean;
  children: ReactNode;
}) {
  const look = danger
    ? "bg-danger text-white"
    : secondary
      ? "bg-glass text-ink shadow-[0_0_0_1px_var(--color-edge)] hover:shadow-[0_0_0_1px_var(--color-dim)]"
      : "bg-ink text-on-ink shadow-[0_6px_16px_-6px_rgb(14_18_24/0.6)]";
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`min-w-28 rounded-full px-5 py-2 text-[13px] font-semibold transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-30 ${look}`}
    >
      {children}
    </button>
  );
}

function TitleBar() {
  const t = useT();
  return (
    <div data-tauri-drag-region className="flex h-9 shrink-0 justify-end">
      <TitleButton label={t("minimize")} onClick={minimizeWindow}>
        <Minus className="size-4" strokeWidth={1.5} />
      </TitleButton>
      <TitleButton label={t("close")} onClick={closeWindow} danger>
        <X className="size-4" strokeWidth={1.5} />
      </TitleButton>
    </div>
  );
}

function TitleButton({
  label,
  onClick,
  danger,
  children,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className={`grid w-11 place-items-center text-dim transition-colors ${
        danger ? "hover:bg-[#c42b1c] hover:text-white" : "hover:bg-edge hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}
