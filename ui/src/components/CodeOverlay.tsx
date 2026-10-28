import { useState } from "react";
import { useT } from "../i18n";

/** The code a device asks for, large enough to read from across the desk.
    It stays until the device is done with it (or it expires); "Hide" only
    covers this code, and the next one appears again. */
export function CodeOverlay({ code }: { code: string | null }) {
  const t = useT();
  const [hidden, setHidden] = useState<string | null>(null);
  if (!code || code === hidden) return null;

  return (
    <div
      role="alertdialog"
      aria-labelledby="code-title"
      aria-describedby="code-hint"
      className="fixed inset-0 z-30 grid place-items-center bg-mist/85 p-6 backdrop-blur-md"
      style={{ animation: "pop-in 160ms ease-out" }}
    >
      <div className="flex flex-col items-center text-center">
        <p id="code-title" className="text-xs font-semibold tracking-wide text-dim uppercase">
          {t("code.title")}
        </p>
        <div className="mt-5 flex flex-col items-center" aria-label={code.split("").join(" ")}>
          <Digits code={code} />
          <div className="bg-iris mt-2 h-px w-56" />
          <div className="reflection flex h-9 items-end overflow-hidden opacity-20" aria-hidden>
            <Digits code={code} />
          </div>
        </div>
        <p id="code-hint" className="mt-3 max-w-64 text-[13px] text-ink-soft">
          {t("code.hint")}
        </p>
        <button
          type="button"
          onClick={() => setHidden(code)}
          className="mt-6 rounded-full bg-glass px-5 py-2 text-[13px] font-semibold shadow-[0_0_0_1px_var(--color-edge)] transition hover:shadow-[0_0_0_1px_var(--color-dim)]"
        >
          {t("code.hide")}
        </button>
      </div>
    </div>
  );
}

function Digits({ code }: { code: string }) {
  return (
    <p className="font-display text-5xl leading-none tracking-[0.3em] tabular-nums" style={{ marginRight: "-0.3em" }}>
      {code}
    </p>
  );
}
