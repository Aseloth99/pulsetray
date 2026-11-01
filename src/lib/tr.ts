import { t, type Key } from "./i18n";
import { duration } from "./format";
import { settings } from "./settings.svelte";

/** Translate in the currently selected language (reactive when called from a component template). */
export const tr = (key: Key): string => t(settings.lang, key);

/** Uptime-style duration with day/hour/minute letters of the selected language (4d 21h / 4g 21s). */
export const dur = (secs: number): string =>
  duration(secs, settings.lang === "tr" ? { d: "g", h: "s", m: "dk" } : { d: "d", h: "h", m: "m" });

/** Lower case by the rules of the selected language: plain toLowerCase made "İşlem" "i̇şlem" (a dotted i plus a
 *  combining dot, shown as "İşlem") and would turn "I" into "i" instead of "ı". */
export const lower = (s: string): string => s.toLocaleLowerCase(settings.lang === "tr" ? "tr-TR" : "en-US");
