import { useEffect, useState, type ReactNode } from "react";
import { Check } from "lucide-react";
import {
  checkForUpdates,
  forgetPairedDevices,
  MIN_PASSWORD_LEN,
  saveSettings,
  type Resolution,
  type SecurityMode,
  type Settings,
  type Snapshot,
} from "../backend";
import { languages, useT, type MessageKey } from "../i18n";
import { Select } from "./Select";

/** Fixed sizes; "auto" (the monitor's size) is added with translated text. */
const resolutions: { id: Resolution; label: string; size: string; height: number }[] = [
  { id: "1080p", label: "Full HD", size: "1920×1080", height: 1080 },
  { id: "1440p", label: "QHD", size: "2560×1440", height: 1440 },
  { id: "2160p", label: "4K", size: "3840×2160", height: 2160 },
];

/** What senders see: changing it restarts the receiver, so it waits for "Save". */
type ReceiverDraft = Pick<Settings, "name" | "resolution" | "hevc" | "security" | "password">;

export function SettingsPanel({ snapshot }: { snapshot: Snapshot }) {
  return (
    <div className="space-y-8">
      <ReceiverSettings snapshot={snapshot} />
      <hr className="border-edge" />
      <AppSettings settings={snapshot.settings} />
    </div>
  );
}

function ReceiverSettings({ snapshot }: { snapshot: Snapshot }) {
  const t = useT();
  const { name, resolution, hevc, security, password } = snapshot.settings;
  const [draft, setDraft] = useState<ReceiverDraft>({ name, resolution, hevc, security, password });
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  // Follow changes made elsewhere (e.g. after saving).
  useEffect(
    () => setDraft({ name, resolution, hevc, security, password }),
    [name, resolution, hevc, security, password],
  );

  const dirty =
    draft.name !== name ||
    draft.resolution !== resolution ||
    draft.hevc !== hevc ||
    draft.security !== security ||
    (draft.security === "password" && draft.password !== password);
  const valid =
    draft.name.trim() !== "" && (draft.security !== "password" || draft.password.length >= MIN_PASSWORD_LEN);
  const hevcOn = draft.hevc && snapshot.hevcAvailable;
  const update = (change: Partial<ReceiverDraft>) => {
    setDraft((d) => ({ ...d, ...change }));
    setMessage(null);
  };

  async function save() {
    setSaving(true);
    try {
      await saveSettings({ ...snapshot.settings, ...draft });
      setMessage({ ok: true, text: t("settings.saved") });
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="space-y-6">
      <GroupTitle>{t("settings.receiver")}</GroupTitle>

      <Section title={t("settings.name")} hint={t("settings.nameHint")}>
        <input
          id="receiver-name"
          value={draft.name}
          maxLength={50}
          onChange={(e) => update({ name: e.target.value })}
          className="w-full rounded-xl bg-glass px-3 py-2 text-[13px] shadow-[0_0_0_1px_var(--color-edge)] outline-none focus:shadow-[0_0_0_1.5px_var(--color-ink)]"
        />
      </Section>

      <Section
        title={t("settings.quality")}
        hint={t(hevcOn ? "settings.qualityHint" : "settings.qualityLimited")}
      >
        <Choices
          value={draft.resolution}
          onChange={(resolution) => update({ resolution })}
          options={[
            { value: "auto" as Resolution, label: t("settings.auto"), detail: t("settings.autoDetail") },
            ...resolutions.map((r) => ({
              value: r.id,
              label: r.label,
              detail: !hevcOn && r.height > 1080 ? t("settings.needsHevc") : r.size,
              mono: hevcOn || r.height <= 1080,
            })),
          ]}
        />
      </Section>

      <Section title={t("settings.codec")}>
        <label className="flex cursor-pointer items-start gap-3">
          <Toggle checked={hevcOn} disabled={!snapshot.hevcAvailable} onChange={(hevc) => update({ hevc })} />
          <span className="text-[13px]">
            <span className="block font-medium">{t("settings.hevc")}</span>
            <span className="block text-dim">
              {t(snapshot.hevcAvailable ? "settings.hevcOn" : "settings.hevcMissing")}
            </span>
          </span>
        </label>
      </Section>

      <Section title={t("settings.security")}>
        <Choices
          value={draft.security}
          onChange={(security) => update({ security })}
          options={securityModes.map((mode) => ({ value: mode.id, label: t(mode.label), detail: t(mode.detail) }))}
        />
        {draft.security === "password" && (
          <input
            id="password"
            type="password"
            value={draft.password}
            maxLength={64}
            placeholder={t("settings.passwordPlaceholder")}
            onChange={(e) => update({ password: e.target.value })}
            className="mt-2 w-full rounded-xl bg-glass px-3 py-2 text-[13px] shadow-[0_0_0_1px_var(--color-edge)] outline-none placeholder:text-faint focus:shadow-[0_0_0_1.5px_var(--color-ink)]"
          />
        )}
        {security === "code" && draft.security === "code" && <ForgetDevices />}
      </Section>

      <div className="space-y-2">
        <div className="flex items-center gap-3">
          <button
            disabled={!dirty || saving || !valid}
            onClick={save}
            className="rounded-full bg-ink px-5 py-2 text-[13px] font-semibold text-on-ink transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-30"
          >
            {t(saving ? "settings.saving" : "settings.save")}
          </button>
          {message && (
            <span className={`flex items-center gap-1.5 text-[13px] ${message.ok ? "text-ready" : "text-danger"}`}>
              {message.ok && <Check className="size-4" />}
              {message.text}
            </span>
          )}
        </div>
        <p className="text-xs text-faint">{t("settings.restartNote")}</p>
      </div>
    </div>
  );
}

const securityModes: { id: SecurityMode; label: MessageKey; detail: MessageKey }[] = [
  { id: "none", label: "settings.securityNone", detail: "settings.securityNoneDetail" },
  { id: "password", label: "settings.securityPassword", detail: "settings.securityPasswordDetail" },
  { id: "code", label: "settings.securityCode", detail: "settings.securityCodeDetail" },
  { id: "codeEveryTime", label: "settings.securityEveryTime", detail: "settings.securityEveryTimeDetail" },
];

/** Devices paired with a code skip it next time; this makes them ask again. */
function ForgetDevices() {
  const t = useT();
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      disabled={done}
      onClick={async () => {
        await forgetPairedDevices().catch(() => {});
        setDone(true);
      }}
      className="mt-2 text-xs font-medium text-dim underline-offset-2 hover:text-ink hover:underline disabled:no-underline"
    >
      {t(done ? "settings.devicesForgotten" : "settings.forgetDevices")}
    </button>
  );
}

/** Preferences of the app itself: applied as soon as they change. */
function AppSettings({ settings }: { settings: Settings }) {
  const t = useT();
  const [error, setError] = useState<string | null>(null);
  const apply = (change: Partial<Settings>) => {
    setError(null);
    saveSettings({ ...settings, ...change }).catch((e) => setError(String(e)));
  };

  return (
    <div className="space-y-6">
      <GroupTitle>{t("settings.app")}</GroupTitle>

      <Section title={t("settings.language")}>
        <Select
          id="language"
          value={settings.language}
          onChange={(language) => apply({ language })}
          options={languages.map((l) => ({ value: l.code as string, label: l.label }))}
        />
      </Section>

      <Section title={t("settings.closeMain")}>
        <Choices
          value={settings.closeAction}
          onChange={(closeAction) => apply({ closeAction })}
          options={[
            { value: "quit", label: t("settings.closeQuit"), detail: t("settings.closeQuitDetail") },
            { value: "tray", label: t("settings.closeTray"), detail: t("settings.closeTrayDetail") },
          ]}
        />
      </Section>

      <Section title={t("settings.closeMirror")}>
        <Choices
          value={settings.mirrorCloseAction}
          onChange={(mirrorCloseAction) => apply({ mirrorCloseAction })}
          options={[
            { value: "disconnect", label: t("settings.mirrorDisconnect"), detail: t("settings.mirrorDisconnectDetail") },
            { value: "hide", label: t("settings.mirrorHide"), detail: t("settings.mirrorHideDetail") },
          ]}
        />
      </Section>

      <Section title={t("settings.updates")}>
        <label className="flex cursor-pointer items-start gap-3">
          <Toggle checked={settings.checkUpdates} onChange={(checkUpdates) => apply({ checkUpdates })} />
          <span className="text-[13px]">
            <span className="block font-medium">{t("settings.checkUpdates")}</span>
            <span className="block text-dim">{t("settings.checkUpdatesDetail")}</span>
          </span>
        </label>
        <CheckNow />
      </Section>

      {error && <p className="text-[13px] text-danger">{error}</p>}
    </div>
  );
}

/** "Check now": a newer version shows up as the notice at the top. */
function CheckNow() {
  const t = useT();
  const [checking, setChecking] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  return (
    <div className="mt-3 flex items-center gap-3">
      <button
        type="button"
        disabled={checking}
        onClick={async () => {
          setChecking(true);
          setStatus(null);
          try {
            const newer = await checkForUpdates();
            if (!newer) setStatus({ ok: true, text: t("settings.upToDate") });
          } catch (e) {
            setStatus({ ok: false, text: t("settings.checkFailed", { error: String(e) }) });
          } finally {
            setChecking(false);
          }
        }}
        className="rounded-full bg-glass px-4 py-1.5 text-xs font-semibold shadow-[0_0_0_1px_var(--color-edge)] transition hover:shadow-[0_0_0_1px_var(--color-dim)] disabled:opacity-60"
      >
        {t(checking ? "settings.checking" : "settings.checkNow")}
      </button>
      {status && (
        <span className={`flex items-center gap-1.5 text-xs ${status.ok ? "text-ready" : "text-danger"}`}>
          {status.ok && <Check className="size-3.5" />}
          {status.text}
        </span>
      )}
    </div>
  );
}

function GroupTitle({ children }: { children: ReactNode }) {
  return <h2 className="text-[15px] font-semibold tracking-tight">{children}</h2>;
}

function Section({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <section>
      <h3 className="mb-2 text-xs font-semibold tracking-wide text-dim uppercase">{title}</h3>
      {children}
      {hint && <p className="mt-2 text-xs text-faint">{hint}</p>}
    </section>
  );
}

/** A row of mutually exclusive tiles, each with a label and a detail line. */
function Choices<T extends string>({
  value,
  onChange,
  options,
}: {
  value: T;
  onChange: (value: T) => void;
  options: { value: T; label: string; detail: string; mono?: boolean }[];
}) {
  return (
    <div role="radiogroup" className="grid grid-cols-2 gap-2">
      {options.map((o) => {
        const selected = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={selected}
            onClick={() => onChange(o.value)}
            className={`rounded-xl bg-glass p-3 text-left transition-shadow ${
              selected
                ? "shadow-[0_0_0_1.5px_var(--color-ink)]"
                : "shadow-[0_0_0_1px_var(--color-edge)] hover:shadow-[0_0_0_1px_var(--color-dim)]"
            }`}
          >
            <p className="text-[13px] font-semibold">{o.label}</p>
            <p className={`text-dim ${o.mono ? "font-mono text-[11px]" : "text-xs leading-snug"}`}>{o.detail}</p>
          </button>
        );
      })}
    </div>
  );
}

function Toggle({
  checked,
  disabled,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative mt-0.5 h-6 w-11 shrink-0 rounded-full transition disabled:opacity-40 ${
        checked ? "bg-ink" : "bg-faint/45"
      }`}
    >
      <span
        className={`absolute top-0.5 left-0.5 size-5 rounded-full shadow transition-transform ${
          checked ? "translate-x-5 bg-on-ink" : "bg-white"
        }`}
      />
    </button>
  );
}
