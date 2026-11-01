import test from "node:test";
import assert from "node:assert/strict";
import { lib, read } from "./load.mjs";

const { menuRows } = await lib("traymenu");
const { dict } = await lib("i18n");

const base = { offer: null, popupOpen: false, theme: "system", themeOpen: false, hudOn: false };
/** "update show settings theme | hud | quit": the entry ids with a "|" for every separator. */
const shape = (s) =>
  menuRows({ ...base, ...s })
    .map((r) => (r.kind === "sep" ? "|" : r.id))
    .join(" ");

test("the order: Show, Settings, Theme, separator, Mini Gauge, separator, Exit", () => {
  assert.equal(shape({}), "show settings theme | hud | quit");
});

test("an update on offer goes first, highlighted, with its own separator", () => {
  assert.equal(shape({ offer: "1.2.0" }), "update | show settings theme | hud | quit");
  const [first] = menuRows({ ...base, offer: "1.2.0" });
  assert.equal(first.version, "1.2.0");
  assert.equal(first.strong, true);
  assert.equal(first.label, "menuUpdate");
});

test("Show becomes Hide while a pinned panel is open", () => {
  assert.equal(shape({ popupOpen: true }), "hide settings theme | hud | quit");
  assert.equal(menuRows({ ...base, popupOpen: true })[0].label, "menuHide");
  assert.equal(menuRows(base)[0].label, "menuShow");
});

test("the Theme list opens in place: System, Dark, Light, the one in use checked", () => {
  assert.equal(shape({ themeOpen: true }), "show settings theme theme:system theme:dark theme:light | hud | quit");
  for (const theme of ["system", "dark", "light"]) {
    const checked = menuRows({ ...base, themeOpen: true, theme })
      .filter((r) => r.kind === "item" && r.sub && r.check)
      .map((r) => r.id);
    assert.deepEqual(checked, [`theme:${theme}`]);
  }
  const themeRow = menuRows({ ...base, themeOpen: true }).find((r) => r.id === "theme");
  assert.equal(themeRow.expandable, true);
  assert.equal(themeRow.expanded, true);
  assert.equal(menuRows(base).find((r) => r.id === "theme").expanded, false);
});

test("the Mini Gauge entry shows its state; Exit is the danger entry, last", () => {
  assert.equal(menuRows({ ...base, hudOn: true }).find((r) => r.id === "hud").check, true);
  assert.equal(menuRows(base).find((r) => r.id === "hud").check, false);
  const last = menuRows(base).at(-1);
  assert.equal(last.id, "quit");
  assert.equal(last.danger, true);
});

test("there is no Language entry in any state", () => {
  for (const s of [{}, { offer: "2.0.0" }, { themeOpen: true }, { popupOpen: true, themeOpen: true, hudOn: true }]) {
    for (const r of menuRows({ ...base, ...s })) {
      if (r.kind === "item") assert.doesNotMatch(r.id + r.label, /lang/i, JSON.stringify(r));
    }
  }
});

test("every label is a real key of both languages, and no entry is unlabelled", () => {
  for (const r of menuRows({ ...base, offer: "1.0.1", themeOpen: true, popupOpen: true })) {
    if (r.kind !== "item") continue;
    assert.ok(r.label in dict.en && r.label in dict.tr, r.label);
    assert.ok(r.icon, `${r.id} has no icon`);
  }
});

test("the ids the page sends are the ones Rust acts on (tray_menu_action in lib.rs)", () => {
  const rs = read("src-tauri/src/lib.rs");
  const actions = rs.slice(rs.indexOf("fn tray_menu_action"), rs.indexOf("// ------------------------------------------------------------------ desktop mini gauge"));
  for (const id of ["show", "hide", "settings", "update", "quit", "hud"]) assert.match(actions, new RegExp(`"${id}"`), id);
  assert.match(actions, /theme_from_menu_id/);
  assert.match(actions, /"theme-chosen"/);
  assert.doesNotMatch(actions, /"language/);
  // and the main window applies what Rust tells it
  assert.match(read("src/routes/+page.svelte"), /listen<string>\("theme-chosen"/);
});
