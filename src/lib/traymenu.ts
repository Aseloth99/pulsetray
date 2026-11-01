/**
 * The tray icon's right-click menu as data (routes/menu/+page.svelte draws it, tests/traymenu.test.mjs checks the order).
 *
 * The same skeleton as the other apps of the family:
 *   [Update · vX]  ·  Show / Hide  ·  Settings  ·  Theme ▸ (System, Dark, Light)  ·  --  ·  Mini Gauge  ·  --  ·  Exit
 * There is deliberately no Language entry (the language is rarely changed; Settings has it). The theme list opens in
 * place (this menu is a small page of its own, not a native menu with sub-menus).
 */
import type { Key } from "./i18n";
import { THEME_MODES, type ThemeMode } from "./prefs";

export type MenuAction =
  | "update"
  | "show"
  | "hide"
  | "settings"
  | "theme"
  | `theme:${ThemeMode}`
  | "hud"
  | "quit";

export type MenuRow =
  | { kind: "sep" }
  | {
      kind: "item";
      id: MenuAction;
      icon: string;
      label: Key;
      /** The update entry: the version that goes into its label ("Update · v1.2.3"). */
      version?: string;
      /** Highlighted (the update entry). */
      strong?: boolean;
      /** Shows a check mark (the mini gauge being on, the theme in use). */
      check?: boolean;
      danger?: boolean;
      /** The Theme row: a click opens / closes the list instead of choosing something. */
      expandable?: boolean;
      expanded?: boolean;
      /** One of the theme choices under the Theme row. */
      sub?: boolean;
    };

export interface MenuState {
  /** The version of the update on offer, or null. */
  offer: string | null;
  /** The panel is open and stays open (a pinned panel): the entry then hides it, otherwise it shows it. */
  popupOpen: boolean;
  theme: ThemeMode;
  themeOpen: boolean;
  hudOn: boolean;
}

const THEME_KEY: Record<ThemeMode, Key> = { system: "themeSystem", dark: "themeDark", light: "themeLight" };
const THEME_ICON: Record<ThemeMode, string> = { system: "system", dark: "moon", light: "sun" };

export function menuRows(s: MenuState): MenuRow[] {
  const rows: MenuRow[] = [];
  if (s.offer) {
    rows.push({ kind: "item", id: "update", icon: "update", label: "menuUpdate", version: s.offer, strong: true });
    rows.push({ kind: "sep" });
  }
  rows.push({ kind: "item", id: s.popupOpen ? "hide" : "show", icon: "panel", label: s.popupOpen ? "menuHide" : "menuShow" });
  rows.push({ kind: "item", id: "settings", icon: "gear", label: "settings" });
  rows.push({ kind: "item", id: "theme", icon: "contrast", label: "menuTheme", expandable: true, expanded: s.themeOpen });
  if (s.themeOpen) {
    for (const mode of THEME_MODES) {
      rows.push({ kind: "item", id: `theme:${mode}`, icon: THEME_ICON[mode], label: THEME_KEY[mode], check: s.theme === mode, sub: true });
    }
  }
  rows.push({ kind: "sep" });
  rows.push({ kind: "item", id: "hud", icon: "gauge", label: "hudMenu", check: s.hudOn });
  rows.push({ kind: "sep" });
  rows.push({ kind: "item", id: "quit", icon: "power", label: "menuQuit", danger: true });
  return rows;
}
