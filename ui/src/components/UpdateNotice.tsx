import { useState } from "react";
import { ArrowDownToLine } from "lucide-react";
import { installUpdate, remindUpdateLater, skipUpdate, type UpdateInfo } from "../backend";
import { useT } from "../i18n";

/** A newer version is out: what changed, and what to do about it. */
export function UpdateNotice({ update, current }: { update: UpdateInfo; current: string }) {
  const t = useT();
  const [showNotes, setShowNotes] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const notes = plainText(update.notes);

  async function install() {
    setBusy(true);
    setError(null);
    try {
      await installUpdate();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      className="mb-5 rounded-2xl bg-glass p-4 shadow-[0_0_0_1px_var(--color-edge),0_10px_24px_-14px_rgb(0_0_0/0.35)]"
      style={{ animation: "pop-in 200ms ease-out" }}
    >
      <div className="flex items-start gap-3">
        <span className="bg-iris grid size-8 shrink-0 place-items-center rounded-full text-white">
          <ArrowDownToLine className="size-4" strokeWidth={2} />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-[13px] font-semibold">{t("update.available", { version: update.version })}</p>
          <p className="text-xs text-dim">
            {t("update.current", { version: current })}
            {notes && (
              <button
                type="button"
                onClick={() => setShowNotes(!showNotes)}
                aria-expanded={showNotes}
                className="ml-1.5 font-medium text-ink-soft hover:text-ink hover:underline"
              >
                {t("update.whatsNew")} {showNotes ? "▴" : "▾"}
              </button>
            )}
          </p>
        </div>
      </div>

      {showNotes && (
        <div className="mt-3 max-h-40 overflow-y-auto rounded-lg bg-mist/70 px-3 py-2 text-xs leading-relaxed whitespace-pre-line text-ink-soft">
          {notes}
        </div>
      )}
      {error && <p className="mt-2 text-xs text-danger">{t("update.failed", { error })}</p>}

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={install}
          className="rounded-full bg-ink px-4 py-1.5 text-xs font-semibold text-on-ink transition hover:opacity-90 disabled:opacity-60"
        >
          {t(busy ? "update.downloading" : update.installable ? "update.install" : "update.download")}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => remindUpdateLater()}
          className="rounded-full bg-glass px-4 py-1.5 text-xs font-semibold shadow-[0_0_0_1px_var(--color-edge)] transition hover:shadow-[0_0_0_1px_var(--color-dim)]"
        >
          {t("update.later")}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => skipUpdate()}
          className="ml-auto text-xs text-faint transition hover:text-ink"
        >
          {t("update.skip")}
        </button>
      </div>
    </section>
  );
}

/** Release notes are Markdown written for GitHub: keep the readable text and
    the bullet points, drop tables, notes, code fences and link syntax. */
function plainText(markdown: string): string {
  return markdown
    .split(/\r?\n/)
    .filter((line) => !/^\s*(\||>|```)/.test(line))
    .map((line) =>
      line
        .replace(/^#+\s*/, "")
        .replace(/^\s*[-*]\s+/, "• ")
        .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
        .replace(/[*_`]/g, ""),
    )
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
