import test from "node:test";
import assert from "node:assert/strict";
import { lib, read } from "./load.mjs";

const { tileMenuRows, tileCopyText } = await lib("tilemenu");
const { menuRows } = await lib("traymenu");
const { dict } = await lib("i18n");

test("a tile's right-click menu: Copy Values, Hide This Module, separator, Edit Modules", () => {
  const rows = tileMenuRows();
  assert.deepEqual(
    rows.map((r) => (r.kind === "sep" ? "--" : r.id)),
    ["copy", "hide", "--", "edit"],
  );
  for (const r of rows.filter((x) => x.kind === "item")) {
    assert.ok(r.label in dict.en && r.label in dict.tr, r.label);
    assert.ok(r.tone, `${r.id} has a colour`);
  }
  assert.equal(dict.tr.tileCopy, "Değerleri Kopyala");
  assert.equal(dict.tr.tileHide, "Bu Modülü Gizle");
  assert.equal(dict.tr.tileEdit, "Modülleri Düzenle");
});

test("every icon the menus use exists", () => {
  const icons = read("src/lib/Icon.svelte");
  const rows = [
    ...tileMenuRows(),
    ...menuRows({ offer: "9.9.9", popupOpen: false, theme: "dark", themeOpen: true, hudOn: false }),
  ];
  for (const r of rows.filter((x) => x.kind === "item")) {
    assert.match(icons, new RegExp(`\\n\\s+${r.icon}:`), r.icon);
  }
  for (const name of ["folder", "x"]) assert.match(icons, new RegExp(`\\n\\s+${name}:`), name);
});

test("Copy Values: one reading per line, the title first, no blank or repeated lines", () => {
  const raw = "CPU\n\n  23%  \n23%\nCores   8\nTemperature 54 °C\n";
  assert.equal(tileCopyText(raw), "CPU\n23%\nCores 8\nTemperature 54 °C");
});

test("Copy Values leaves out the tabs that are not selected", () => {
  const raw = "Processes\nCPU\nGPU\nRAM\nchrome\n12.3%\nCode\n4.1%";
  assert.equal(tileCopyText(raw, ["GPU", "RAM"]), "Processes\nCPU\nchrome\n12.3%\nCode\n4.1%");
  // a chart's label line: the chevron and the lone separators go, "Add chart" is skipped, the readings stay
  const chart = "Memory\nRAM\n\u2304\n100%\navg 42% · peak 47%\n· 6 s ago\nAdd chart\nUsed\n14.6 GB";
  assert.equal(tileCopyText(chart, ["Add chart"]), "Memory\nRAM\n100%\navg 42% · peak 47%\n6 s ago\nUsed\n14.6 GB");
});

test("the tray menu's icons are coloured by meaning, like the other apps (the Theme rows stay neutral)", () => {
  const rows = menuRows({ offer: "1.2.3", popupOpen: false, theme: "dark", themeOpen: true, hudOn: true }).filter(
    (r) => r.kind === "item",
  );
  const tone = Object.fromEntries(rows.map((r) => [r.id, r.tone]));
  assert.equal(tone.update, "bat");
  assert.equal(tone.show, "cpu");
  assert.equal(tone.settings, "mem");
  assert.equal(tone.hud, "net");
  assert.equal(tone.quit, "danger");
  assert.equal(tone.theme, undefined);
  for (const r of rows.filter((x) => x.sub)) assert.equal(r.tone, undefined, r.id);
});

test("every tone is a colour app.css defines in both themes", () => {
  const css = read("src/app.css");
  const tones = new Set([
    ...tileMenuRows().filter((r) => r.kind === "item").map((r) => r.tone),
    ...menuRows({ offer: "1.2.3", popupOpen: true, theme: "light", themeOpen: false, hudOn: false })
      .filter((r) => r.kind === "item" && r.tone)
      .map((r) => r.tone),
  ]);
  for (const t of tones) assert.match(css, new RegExp(`--${t}:`), t);
});

test("the main view wires the menu to every tile and skips a process row's own menu", () => {
  const page = read("src/routes/+page.svelte");
  assert.match(page, /data-tile=\{id\}[^>]*oncontextmenu=\{\(e\) => openTileMenu\(e, id\)\}/);
  assert.match(page, /if \(e\.defaultPrevented\) return;/);
  assert.match(page, /bind:this=\{modulesEl\}/);
});
