import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { I18nProvider, languages } from "../i18n";
import { SetupApp } from "./SetupApp";
import "../index.css";

/** The installer speaks the language of Windows, if Riflesso X has it. */
function systemLanguage(): string {
  const codes: string[] = languages.map((l) => l.code);
  for (const tag of navigator.languages ?? [navigator.language]) {
    const exact = codes.find((code) => code.toLowerCase() === tag.toLowerCase());
    const base = codes.find((code) => code.split("-")[0] === tag.split("-")[0].toLowerCase());
    if (exact ?? base) return (exact ?? base)!;
  }
  return "en";
}

const language = systemLanguage();
document.documentElement.lang = language;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <I18nProvider language={language}>
      <SetupApp />
    </I18nProvider>
  </StrictMode>,
);
