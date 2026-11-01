/**
 * Small pure rules behind the settings (theme order, language choice, the two update boxes). Kept apart from
 * settings.svelte.ts (which needs the Svelte compiler) so that tests/ can run them with plain `node --test`.
 */
import type { Lang } from "./i18n";

/** The theme choices, in the order every app of the family lists them: System, Dark, Light. */
export type ThemeMode = "system" | "dark" | "light";
export const THEME_MODES: readonly ThemeMode[] = ["system", "dark", "light"];

/** What is stored as the language: "auto" follows the system language (looked up at start), "tr" / "en" are fixed. */
export type LangPref = "auto" | Lang;
export const LANG_PREFS: readonly LangPref[] = ["auto", "tr", "en"];

/** The languages' own names; the code in front is the same short form the other apps show ("TR · Türkçe", "US · English"). */
const LANGUAGE_NAMES: Record<Lang, string> = { tr: "Türkçe", en: "English" };
const LANGUAGE_CODES: Record<Lang, string> = { tr: "TR", en: "US" };

/** The language the system is set to: Turkish, or English for everything else. `navigator.language` is the Windows display language. */
export function detectSystemLang(navigatorLanguage: string | null | undefined): Lang {
  return (navigatorLanguage ?? "").toLowerCase().startsWith("tr") ? "tr" : "en";
}

/** A stored value as a language choice; anything unknown becomes `fallback` (a record from an older version keeps its "tr" / "en"). */
export function normalizeLangPref(raw: unknown, fallback: LangPref = "auto"): LangPref {
  return raw === "auto" || raw === "tr" || raw === "en" ? raw : fallback;
}

/** The language the UI is actually shown in. */
export function resolveLang(pref: LangPref, system: Lang): Lang {
  return pref === "auto" ? system : pref;
}

/** "TR · Türkçe" / "US · English". */
export function languageLabel(code: Lang): string {
  return `${LANGUAGE_CODES[code]} · ${LANGUAGE_NAMES[code]}`;
}

/**
 * The label of a language choice, built like the other apps do: the fixed languages by their own name, the automatic one
 * as "Otomatik (Sistem - TR · Türkçe)" (the base text ends with ")" and the detected language is put in front of it).
 */
export function langChoiceLabel(pref: LangPref, autoBase: string, system: Lang): string {
  if (pref !== "auto") return languageLabel(pref);
  const detected = languageLabel(system);
  return autoBase.endsWith(")") ? `${autoBase.slice(0, -1)} - ${detected})` : `${autoBase} - ${detected}`;
}

/** A stored value as a theme choice (unknown = `fallback`). */
export function normalizeTheme(raw: unknown, fallback: ThemeMode = "system"): ThemeMode {
  return THEME_MODES.includes(raw as ThemeMode) ? (raw as ThemeMode) : fallback;
}

/**
 * "Install Updates Automatically" only counts while "Check for Updates Automatically" is on: with the check off nothing
 * is found in the background, and the box is greyed out in Settings (its own value is kept for when the check comes back).
 */
export function autoInstallActive(autoCheck: boolean, autoInstall: boolean): boolean {
  return autoCheck && autoInstall;
}
