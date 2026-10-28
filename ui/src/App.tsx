import { useEffect, useState } from "react";
import { useSnapshot, type Snapshot } from "./backend";
import { I18nProvider, useT, type MessageKey } from "./i18n";
import { Header } from "./components/Header";
import { DevicesPanel } from "./components/DevicesPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { CodeOverlay } from "./components/CodeOverlay";
import { UpdateNotice } from "./components/UpdateNotice";

type Tab = "devices" | "settings";

const tabs: { id: Tab; label: MessageKey }[] = [
  { id: "devices", label: "tab.devices" },
  { id: "settings", label: "tab.settings" },
];

export default function App() {
  const snapshot = useSnapshot();
  const language = snapshot?.settings.language ?? "en";

  // Lets the browser pick the right fallback fonts (CJK, Cyrillic).
  useEffect(() => {
    document.documentElement.lang = language;
  }, [language]);

  return (
    <I18nProvider language={language}>
      {snapshot ? <Main snapshot={snapshot} /> : <Loading />}
    </I18nProvider>
  );
}

function Loading() {
  const t = useT();
  return <div className="grid h-full place-items-center text-sm text-faint">{t("starting")}</div>;
}

function Main({ snapshot }: { snapshot: Snapshot }) {
  const t = useT();
  const [tab, setTab] = useState<Tab>("devices");

  return (
    <div className="flex h-full flex-col">
      <Header snapshot={snapshot} />

      <nav role="tablist" className="column flex shrink-0 gap-5 pt-3">
        {tabs.map(({ id, label }) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            onClick={() => setTab(id)}
            className={`relative py-1.5 text-[13px] font-medium transition-colors ${
              tab === id ? "text-ink" : "text-dim hover:text-ink-soft"
            }`}
          >
            {t(label)}
            {tab === id && <span className="bg-iris absolute inset-x-0 -bottom-0.5 h-0.5 rounded-full" />}
          </button>
        ))}
      </nav>

      <main className="flex-1 overflow-y-auto [scrollbar-gutter:stable]">
        <div className="column flex min-h-full flex-col pt-5 pb-4">
          {snapshot.update && <UpdateNotice update={snapshot.update} current={snapshot.version} />}
          {tab === "devices" ? <DevicesPanel snapshot={snapshot} /> : <SettingsPanel snapshot={snapshot} />}
        </div>
      </main>

      <footer className="shrink-0 pb-3 text-center text-[11px] text-faint">Riflesso X {snapshot.version}</footer>

      <CodeOverlay code={snapshot.code} />
    </div>
  );
}
