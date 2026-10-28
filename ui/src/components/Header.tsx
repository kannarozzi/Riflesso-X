import type { ReactNode } from "react";
import { Minus, X } from "lucide-react";
import { closeWindow, minimizeWindow, openRepository, type Snapshot } from "../backend";
import { useT } from "../i18n";

const statusDot = {
  starting: "bg-warn animate-pulse",
  running: "bg-ready",
  error: "bg-danger",
  playback: "bg-iris",
} as const;

/** Title bar and wordmark. The window has no system frame, so the empty
    areas here are where it is dragged from. */
export function Header({ snapshot }: { snapshot: Snapshot }) {
  const t = useT();
  const state = snapshot.status.state;
  return (
    <header data-tauri-drag-region className="shrink-0">
      <div data-tauri-drag-region className="flex h-9 justify-end">
        <WindowButton label={t("github")} onClick={openRepository}>
          <GitHubMark />
        </WindowButton>
        <WindowButton label={t("minimize")} onClick={minimizeWindow}>
          <Minus className="size-4" strokeWidth={1.5} />
        </WindowButton>
        <WindowButton label={t("close")} onClick={closeWindow} danger>
          <X className="size-4" strokeWidth={1.5} />
        </WindowButton>
      </div>

      <div data-tauri-drag-region className="column flex items-start justify-between pt-1">
        <div data-tauri-drag-region aria-label="Riflesso X">
          <Wordmark />
          <Wordmark mirrored />
        </div>
        <span className="flex items-center gap-1.5 rounded-full bg-glass py-1 pr-2.5 pl-2 text-xs font-medium shadow-[0_0_0_1px_var(--color-edge),0_1px_2px_var(--color-edge)]">
          <span className={`size-1.5 rounded-full ${statusDot[state]}`} />
          {t(`status.${state}`)}
        </span>
      </div>
    </header>
  );
}

function Wordmark({ mirrored }: { mirrored?: boolean }) {
  return (
    <p
      data-tauri-drag-region
      aria-hidden={mirrored}
      className={`font-display text-[15px] leading-none tracking-[0.12em] ${mirrored ? "reflection mt-px opacity-20" : ""}`}
    >
      RIFLESSO X
    </p>
  );
}

/** GitHub's logo (lucide no longer ships brand icons). */
function GitHubMark() {
  return (
    <svg viewBox="0 0 16 16" className="size-[15px]" fill="currentColor" aria-hidden>
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
    </svg>
  );
}

function WindowButton({
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
