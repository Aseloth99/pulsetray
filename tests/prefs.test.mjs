import test from "node:test";
import assert from "node:assert/strict";
import { lib, read } from "./load.mjs";

const prefs = await lib("prefs");

test("the theme order is System, Dark, Light", () => {
  assert.deepEqual([...prefs.THEME_MODES], ["system", "dark", "light"]);
});

test("the settings page lists the themes from that one list (and the quick sun / moon button still flips light and dark)", () => {
  const page = read("src/routes/+page.svelte");
  assert.match(page, /THEME_MODES\.map\(/);
  assert.doesNotMatch(page, /\["system", "themeSystem"\],\s*\["light"/, "no hand-written order with Light before Dark");
  assert.match(page, /settings\.theme = theme === "dark" \? "light" : "dark";/, "toggleTheme is unchanged");
});

test("language: system language is Turkish for tr-*, English for everything else", () => {
  assert.equal(prefs.detectSystemLang("tr-TR"), "tr");
  assert.equal(prefs.detectSystemLang("TR"), "tr");
  assert.equal(prefs.detectSystemLang("en-US"), "en");
  assert.equal(prefs.detectSystemLang("de-DE"), "en");
  assert.equal(prefs.detectSystemLang(""), "en");
  assert.equal(prefs.detectSystemLang(undefined), "en");
});

test("language: a saved 'tr' / 'en' is kept, 'auto' is accepted, anything else follows the default", () => {
  assert.equal(prefs.normalizeLangPref("tr"), "tr");
  assert.equal(prefs.normalizeLangPref("en"), "en");
  assert.equal(prefs.normalizeLangPref("auto"), "auto");
  assert.equal(prefs.normalizeLangPref(undefined), "auto", "a new install: automatic");
  assert.equal(prefs.normalizeLangPref("de"), "auto");
  assert.equal(prefs.normalizeLangPref(null, "en"), "en");
});

test("language: 'auto' is resolved to the system language, a fixed choice stays", () => {
  assert.equal(prefs.resolveLang("auto", "tr"), "tr");
  assert.equal(prefs.resolveLang("auto", "en"), "en");
  assert.equal(prefs.resolveLang("tr", "en"), "tr");
  assert.equal(prefs.resolveLang("en", "tr"), "en");
});

test("language: the labels have the format of the other apps (no flags)", () => {
  assert.equal(prefs.languageLabel("tr"), "TR · Türkçe");
  assert.equal(prefs.languageLabel("en"), "US · English");
  assert.equal(prefs.langChoiceLabel("tr", "Otomatik (Sistem)", "en"), "TR · Türkçe");
  assert.equal(prefs.langChoiceLabel("en", "Auto (System)", "tr"), "US · English");
  assert.equal(prefs.langChoiceLabel("auto", "Otomatik (Sistem)", "tr"), "Otomatik (Sistem - TR · Türkçe)");
  assert.equal(prefs.langChoiceLabel("auto", "Auto (System)", "en"), "Auto (System - US · English)");
  assert.equal(prefs.langChoiceLabel("auto", "Auto", "en"), "Auto - US · English");
  for (const l of prefs.LANG_PREFS) assert.doesNotMatch(prefs.langChoiceLabel(l, "Auto (System)", "tr"), /\p{Extended_Pictographic}|[\u{1F1E6}-\u{1F1FF}]/u);
  assert.deepEqual([...prefs.LANG_PREFS], ["auto", "tr", "en"]);
});

test("the settings store: a new install starts on 'auto' with both update boxes on, and an old record keeps its language", () => {
  const src = read("src/lib/settings.svelte.ts");
  const defaults = src.slice(src.indexOf("function defaults()"), src.indexOf("function load()"));
  assert.match(defaults, /lang: "auto"/);
  assert.match(defaults, /autoCheck: true/);
  assert.match(defaults, /autoUpdate: true/);
  const load = src.slice(src.indexOf("function load()"), src.indexOf("class Settings"));
  assert.match(load, /lang: normalizeLangPref\(raw\.lang, base\.lang\)/);
  assert.match(load, /autoCheck: raw\.autoCheck !== false/);
  const save = src.slice(src.indexOf("  save() {"), src.indexOf("get tiles()"));
  assert.match(save, /lang: this\.langPref/, "the choice is stored, not the resolved language");
  assert.match(save, /autoCheck: this\.autoCheck/);
});

test("Install Updates Automatically only counts while Check for Updates Automatically is on", () => {
  assert.equal(prefs.autoInstallActive(true, true), true);
  assert.equal(prefs.autoInstallActive(true, false), false);
  assert.equal(prefs.autoInstallActive(false, true), false);
  assert.equal(prefs.autoInstallActive(false, false), false);
});

test("the backend hears both update boxes (the daily check and the automatic install read them in lib.rs)", () => {
  const page = read("src/routes/+page.svelte");
  assert.match(page, /invoke\("set_auto_update", \{ enabled: settings\.autoUpdate \}\)/);
  assert.match(page, /invoke\("set_auto_check", \{ enabled: settings\.autoCheck \}\)/);
  assert.match(page, /disabled=\{!settings\.autoCheck\}/, "the install box is greyed out without the check");
  const rs = read("src-tauri/src/lib.rs");
  assert.match(rs, /\n\s+set_auto_check,\n/, "the command is registered");
  assert.match(rs, /daily_check_due\(state\.auto_check\.load/);
  assert.match(rs, /automatic_install\(\s*state\.auto_check\.load/);
});
