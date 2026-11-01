import test from "node:test";
import assert from "node:assert/strict";
import { lib } from "./load.mjs";

const { dict } = await lib("i18n");
const en = dict.en;
const tr = dict.tr;

test("Turkish and English have exactly the same keys", () => {
  const onlyEn = Object.keys(en).filter((k) => !(k in tr));
  const onlyTr = Object.keys(tr).filter((k) => !(k in en));
  assert.deepEqual(onlyEn, [], "keys missing in Turkish");
  assert.deepEqual(onlyTr, [], "keys missing in English");
});

test("no empty texts, and both languages use the same {placeholders}", () => {
  // {s} is Turkish only: the ending that follows a number (%2'si, %5'i)
  const holes = (s) => (s.match(/\{[a-z]+\}/gi) ?? []).filter((h) => h !== "{s}").sort().join(",");
  for (const k of Object.keys(en)) {
    assert.ok(en[k].trim() !== "" && tr[k].trim() !== "", `${k} is empty`);
    assert.equal(holes(en[k]), holes(tr[k]), `${k}: placeholders differ`);
  }
});

test("Exit, not Quit: the tray menu entry and the Settings button", () => {
  assert.equal(en.menuQuit, "Exit");
  assert.equal(tr.menuQuit, "Çıkış");
  assert.equal(en.quit, "Exit");
  assert.equal(tr.quit, "Çıkış");
  for (const k of Object.keys(en)) {
    assert.doesNotMatch(en[k], /(?<!\p{L})quit(?!\p{L})/iu, `en.${k}`);
    assert.doesNotMatch(tr[k], /(?<!\p{L})çık(?!\p{L})/iu, `tr.${k}`);
  }
});

test("Start at Login: no 'Windows' in the start-up label (it is not Windows only any more)", () => {
  assert.equal(en.autostart, "Start at Login");
  assert.equal(tr.autostart, "Açılışta Başlat");
  assert.doesNotMatch(en.autostart + tr.autostart, /windows/i);
});

test("the tray menu texts exist in both languages: Show / Hide / Theme, and the theme choices", () => {
  assert.deepEqual([en.menuShow, en.menuHide, en.menuTheme], ["Show", "Hide", "Theme"]);
  assert.deepEqual([tr.menuShow, tr.menuHide, tr.menuTheme], ["Göster", "Gizle", "Tema"]);
  assert.deepEqual([en.themeSystem, en.themeDark, en.themeLight], ["System", "Dark", "Light"]);
  assert.deepEqual([tr.themeSystem, tr.themeDark, tr.themeLight], ["Sistem", "Koyu", "Açık"]);
  assert.ok(!("menuOpen" in en) && !("menuOpen" in tr), "the old 'Open' entry is gone");
});

test("the two update boxes and their one hint", () => {
  assert.equal(en.autoCheck, "Check for Updates Automatically");
  assert.equal(en.autoUpdate, "Install Updates Automatically");
  assert.equal(tr.autoCheck, "Güncellemeleri Otomatik Kontrol Et");
  assert.equal(tr.autoUpdate, "Güncellemeleri Otomatik Kur");
  assert.equal(tr.autoUpdateHint, "Yeni sürüm bulunduğunda arka planda indirilir ve uygulama kullanılmıyorken kurulur.");
  assert.match(en.autoUpdateHint, /background/);
});

test("language: the automatic choice has a base text ending in a bracket (the label adds the detected language)", () => {
  assert.equal(tr.langAuto, "Otomatik (Sistem)");
  assert.equal(en.langAuto, "Auto (System)");
});

// ---- Title Case for control labels (buttons, menu items, headings, switch labels), sentences stay sentences
const EN_SMALL = new Set(["a", "an", "the", "to", "of", "on", "in", "at", "per", "over", "for", "and", "or"]);
const LABELS = [
  "processes", "pingTargets", "resetTargets", "rate", "powerDraw", "coreClock", "memClock", "cpuPower", "chargePower", "timeLeft",
  "perCore", "linkSpeed", "ip", "session", "sinceBoot", "pingStart", "pingStop", "themeToLight", "themeToDark", "advancedMode",
  "pin", "fitContent", "autostart", "autoCheck", "autoUpdate", "checkNow", "updateNotes", "updateLater", "updateRetry", "detailBadge",
  "detailReset", "batHistory", "batHistFirst", "batHistPerYear", "batHistMade", "batHistHealth", "batHistUsage", "batHistCapacity",
  "energyLeft", "batHistCharge", "wifiNearby", "procMulti", "pickTab", "lastUsed", "pickLength", "sortedBy", "alertDisk",
  "alertMemory", "alertTemp", "alertHealth", "alertHog", "alertTest", "downChart", "upChart", "powerChart", "chargeChart", "memCombined",
  "diskModels", "diskHealth", "hudTitle", "hudOn", "hudLocked", "hudHideFs", "hudMenu", "trayIcon", "chartHistory", "appReveal",
  "appEnd", "appEndConfirm", "appEndConfirmN", "menuShow", "menuHide", "menuTheme", "menuQuit", "quit", "swap", "threadCount", "os",
];

/** Words that start with a letter (placeholders like {n}, brackets and numbers are not words). */
const words = (s) => s.split(/[\s/]+/).map((w) => w.replace(/^[("']+/, "")).filter((w) => /^\p{L}/u.test(w));

test("English labels are Title Case", () => {
  for (const k of LABELS) {
    for (const w of words(en[k])) {
      if (EN_SMALL.has(w.toLowerCase()) && w !== words(en[k])[0]) continue;
      // a hyphenated word capitalises each part (Click-Through, Full-Screen, Screen-On)
      for (const part of w.split("-")) {
        assert.equal(part[0], part[0].toLocaleUpperCase("en-US"), `en.${k} "${en[k]}": "${part}"`);
      }
    }
  }
});

test("Turkish labels are Title Case (Turkish capitals: i -> İ, ı -> I)", () => {
  for (const k of LABELS) {
    for (const w of words(tr[k])) {
      if (w === "misiniz?") continue; // the question word of "Emin misiniz? Sonlandır" starts the sentence's second half
      assert.equal(w[0], w[0].toLocaleUpperCase("tr-TR"), `tr.${k} "${tr[k]}": "${w}"`);
    }
  }
});

// ---- "siz": Turkish texts address the user formally
test("Turkish texts use 'siz', never 'sen'", () => {
  // (JS \b only knows ASCII letters, so the word edges are spelled out)
  const word = (w) => new RegExp(`(?<!\\p{L})${w}(?!\\p{L})`, "iu");
  const banned = ["sen", "senin", "seçtiğin", "bağlantın", "kullanıyorsun", "isteğin"].map(word);
  // an imperative in the singular at the end of a sentence ("... tekrar dene.", "... birini aç.")
  const tail = /(?<!\p{L})(aç|bas|dene|kapat|sürükle|ver|et|gir|seç|taşı)\.$/iu;
  for (const [k, v] of Object.entries(tr)) {
    for (const re of banned) assert.doesNotMatch(v, re, `tr.${k}: ${v}`);
    for (const sentence of v.split(/(?<=\.)\s+/)) assert.doesNotMatch(sentence, tail, `tr.${k}: ${sentence}`);
  }
  assert.equal(tr.noTiles, "Tüm modüller gizli. Ayarlar'dan birini açın.");
  assert.equal(tr.upToDate, "En son sürümü kullanıyorsunuz");
});

test("an ellipsis is the single character, never three dots", () => {
  for (const [lang, d] of [["en", en], ["tr", tr]]) {
    for (const [k, v] of Object.entries(d)) assert.ok(!v.includes("..."), `${lang}.${k}: ${v}`);
  }
});

test("the Mini Gauge hint names the switch by its real label", () => {
  assert.ok(en.fpsHideFsHint.includes(`"${en.hudHideFs}"`), en.fpsHideFsHint);
  assert.ok(tr.fpsHideFsHint.includes(`"${tr.hudHideFs}"`), tr.fpsHideFsHint);
});
