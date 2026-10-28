import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Check } from "lucide-react";
import {
  checkForUpdates,
  forgetPairedDevices,
  MIN_PASSWORD_LEN,
  saveSettings,
  type Device,
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

/** What senders see: changing it restarts the receiver, so it is applied as
    soon as it is chosen, after asking if devices would be disconnected. */
type ReceiverFields = Pick<Settings, "name" | "resolution" | "hevc" | "security" | "password">;

/** Same rule as `Settings::receiver_differs` in settings.rs. */
function receiverDiffers(a: Settings, b: Settings) {
  return (
    a.name.trim() !== b.name.trim() ||
    a.resolution !== b.resolution ||
    a.hevc !== b.hevc ||
    a.security !== b.security ||
    (b.security === "password" && a.password !== b.password)
  );
}

type SectionId = "name" | "quality" | "codec" | "security";

/** Where a change shows its outcome: next to the setting that was touched. */
function sectionOf(change: Partial<ReceiverFields>): SectionId {
  if ("name" in change) return "name";
  if ("resolution" in change) return "quality";
  if ("hevc" in change) return "codec";
  return "security";
}

type ApplyStatus = { section: SectionId } & ({ state: "applying" | "saved" } | { state: "error"; text: string });

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
  const saved = snapshot.settings;
  // Text fields are applied when left (or on Enter), not at every key.
  const [name, setName] = useState(saved.name);
  const [password, setPassword] = useState(saved.password);
  // "Password" was picked: it is turned on once one long enough is entered.
  const [choosingPassword, setChoosingPassword] = useState(false);
  // The change being applied, shown before the backend reports it.
  const [applying, setApplying] = useState<Partial<ReceiverFields> | null>(null);
  const [status, setStatus] = useState<ApplyStatus | null>(null);
  const [confirm, setConfirm] = useState<{ answer: (ok: boolean) => void } | null>(null);

  // Changes are applied one at a time, each on top of the previous one.
  const queue = useRef(Promise.resolve());
  const current = useRef(saved);
  const devices = useRef(snapshot.devices);
  useEffect(() => {
    current.current = saved;
    devices.current = snapshot.devices;
  }, [saved, snapshot.devices]);

  useEffect(() => setName(saved.name), [saved.name]);
  useEffect(() => setPassword(saved.password), [saved.password]);
  useEffect(() => setChoosingPassword(false), [saved.security]);
  useEffect(() => {
    if (status?.state !== "saved") return;
    const timer = setTimeout(() => setStatus(null), 3000);
    return () => clearTimeout(timer);
  }, [status]);

  const askToRestart = () => new Promise<boolean>((answer) => setConfirm({ answer }));

  async function apply(change: Partial<ReceiverFields>) {
    const before = current.current;
    const next = { ...before, ...change };
    if (!receiverDiffers(before, next)) return;
    if (devices.current.length > 0 && !(await askToRestart())) {
      if ("name" in change) setName(before.name);
      if ("password" in change) {
        setPassword(before.password);
        setChoosingPassword(false);
      }
      return;
    }
    const section = sectionOf(change);
    setApplying(change);
    setStatus({ section, state: "applying" });
    try {
      await saveSettings(next);
      current.current = next;
      setStatus({ section, state: "saved" });
    } catch (e) {
      setStatus({ section, state: "error", text: String(e) });
    } finally {
      setApplying(null);
    }
  }

  const commit = (change: Partial<ReceiverFields>) => {
    queue.current = queue.current.then(() => apply(change));
  };

  const applyName = () => {
    if (name.trim() === "") setName(saved.name);
    else commit({ name: name.trim() });
  };
  const applyPassword = () => {
    // Too short: it stays here, with the hint below, until it is long enough.
    if (password.length >= MIN_PASSWORD_LEN) commit({ security: "password", password });
  };
  const blurOnEnter = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") e.currentTarget.blur();
  };

  const shown = { ...saved, ...applying };
  const security = choosingPassword ? "password" : shown.security;
  const passwordPending = security === "password" && (saved.security !== "password" || password !== saved.password);
  const hevcOn = shown.hevc && snapshot.hevcAvailable;
  const statusOf = (section: SectionId) => (status?.section === section ? status : undefined);

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <GroupTitle>{t("settings.receiver")}</GroupTitle>
        <p className="text-xs text-faint">{t("settings.restartNote")}</p>
      </div>

      <Section title={t("settings.name")} hint={t("settings.nameHint")} status={statusOf("name")}>
        <input
          id="receiver-name"
          value={name}
          maxLength={50}
          onChange={(e) => setName(e.target.value)}
          onBlur={applyName}
          onKeyDown={blurOnEnter}
          className="w-full rounded-xl bg-glass px-3 py-2 text-[13px] shadow-[0_0_0_1px_var(--color-edge)] outline-none focus:shadow-[0_0_0_1.5px_var(--color-ink)]"
        />
      </Section>

      <Section
        title={t("settings.quality")}
        hint={t(hevcOn ? "settings.qualityHint" : "settings.qualityLimited")}
        status={statusOf("quality")}
      >
        <Choices
          value={shown.resolution}
          onChange={(resolution) => commit({ resolution })}
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

      <Section title={t("settings.codec")} status={statusOf("codec")}>
        <label className="flex cursor-pointer items-start gap-3">
          <Toggle checked={hevcOn} disabled={!snapshot.hevcAvailable} onChange={(hevc) => commit({ hevc })} />
          <span className="text-[13px]">
            <span className="block font-medium">{t("settings.hevc")}</span>
            <span className="block text-dim">
              {t(snapshot.hevcAvailable ? "settings.hevcOn" : "settings.hevcMissing")}
            </span>
          </span>
        </label>
      </Section>

      <Section title={t("settings.security")} status={statusOf("security")}>
        <Choices
          value={security}
          onChange={(mode) => {
            if (mode === "password") {
              if (saved.security !== "password") setChoosingPassword(true);
            } else {
              setChoosingPassword(false);
              commit({ security: mode });
            }
          }}
          options={securityModes.map((mode) => ({ value: mode.id, label: t(mode.label), detail: t(mode.detail) }))}
        />
        {security === "password" && (
          <>
            <input
              id="password"
              type="password"
              value={password}
              maxLength={64}
              autoFocus={choosingPassword}
              placeholder={t("settings.passwordPlaceholder")}
              onChange={(e) => setPassword(e.target.value)}
              onBlur={applyPassword}
              onKeyDown={blurOnEnter}
              className="mt-2 w-full rounded-xl bg-glass px-3 py-2 text-[13px] shadow-[0_0_0_1px_var(--color-edge)] outline-none placeholder:text-faint focus:shadow-[0_0_0_1.5px_var(--color-ink)]"
            />
            {passwordPending && (
              <p className="mt-2 text-xs text-faint">
                {t("settings.passwordPending", { min: MIN_PASSWORD_LEN })}
              </p>
            )}
          </>
        )}
        {saved.security === "code" && security === "code" && <ForgetDevices />}
      </Section>

      {confirm && (
        <ConfirmRestart
          devices={snapshot.devices}
          onAnswer={(ok) => {
            confirm.answer(ok);
            setConfirm(null);
          }}
        />
      )}
    </div>
  );
}

/** Asked before a change that would disconnect the devices in use. */
function ConfirmRestart({ devices, onAnswer }: { devices: Device[]; onAnswer: (ok: boolean) => void }) {
  const t = useT();
  return (
    <div
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="restart-title"
      aria-describedby="restart-hint"
      onKeyDown={(e) => e.key === "Escape" && onAnswer(false)}
      className="fixed inset-0 z-30 grid place-items-center bg-mist/85 p-6 backdrop-blur-md"
      style={{ animation: "pop-in 160ms ease-out" }}
    >
      <div className="flex max-w-72 flex-col items-center text-center">
        <p id="restart-title" className="text-[15px] font-semibold tracking-tight">
          {t("settings.confirmTitle")}
        </p>
        <p id="restart-hint" className="mt-2 text-[13px] text-ink-soft">
          {t("settings.confirmHint")}
        </p>
        {devices.length > 0 && (
          <ul className="mt-3 space-y-0.5 text-[13px] font-medium">
            {devices.map((d) => (
              <li key={d.id}>{d.name}</li>
            ))}
          </ul>
        )}
        <div className="mt-6 flex gap-2">
          <button
            type="button"
            onClick={() => onAnswer(false)}
            className="rounded-full bg-glass px-5 py-2 text-[13px] font-semibold shadow-[0_0_0_1px_var(--color-edge)] transition hover:shadow-[0_0_0_1px_var(--color-dim)]"
          >
            {t("settings.confirmCancel")}
          </button>
          <button
            type="button"
            autoFocus
            onClick={() => onAnswer(true)}
            className="rounded-full bg-ink px-5 py-2 text-[13px] font-semibold text-on-ink transition hover:opacity-90"
          >
            {t("settings.confirmRestart")}
          </button>
        </div>
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

function Section({
  title,
  hint,
  status,
  children,
}: {
  title: string;
  hint?: string;
  /** How the last change made here went. */
  status?: ApplyStatus;
  children: ReactNode;
}) {
  const t = useT();
  return (
    <section>
      <div className="mb-2 flex items-center justify-between gap-3">
        <h3 className="text-xs font-semibold tracking-wide text-dim uppercase">{title}</h3>
        {status?.state === "applying" && <span className="text-xs text-dim">{t("settings.applying")}</span>}
        {status?.state === "saved" && (
          <span className="flex items-center gap-1 text-xs text-ready">
            <Check className="size-3.5" />
            {t("settings.saved")}
          </span>
        )}
      </div>
      {children}
      {status?.state === "error" && <p className="mt-2 text-xs text-danger">{status.text}</p>}
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
