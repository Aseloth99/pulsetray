/**
 * A tile's right-click menu (routes/+page.svelte opens it, TileMenu.svelte draws it, tests/tilemenu.test.mjs checks it):
 *   Copy Values  ·  Hide This Module  ·  --  ·  Edit Modules
 * The process list keeps its own menu on its rows (Open File Location / End Task); a right-click there never opens
 * this one. Icons are coloured by meaning, the same colours as in the tray menu and the other apps of the family.
 */
import type { Key } from "./i18n";

/** Icon colour by meaning (a CSS colour variable of app.css): blue = copy / show, amber = hide, violet = settings. */
export type Tone = "cpu" | "mem" | "net" | "disk" | "bat" | "danger" | "muted";

export type TileAction = "copy" | "hide" | "edit";

export type TileMenuRow = { kind: "sep" } | { kind: "item"; id: TileAction; icon: string; label: Key; tone: Tone };

export function tileMenuRows(): TileMenuRow[] {
  return [
    { kind: "item", id: "copy", icon: "copy", label: "tileCopy", tone: "cpu" },
    { kind: "item", id: "hide", icon: "eyeoff", label: "tileHide", tone: "disk" },
    { kind: "sep" },
    { kind: "item", id: "edit", icon: "sliders", label: "tileEdit", tone: "mem" },
  ];
}

/**
 * The text "Copy Values" puts on the clipboard: what the tile shows, one reading per line, the title first.
 * `raw` is the tile's rendered text (innerText: one line per block); `skip` are texts of its controls (the
 * CPU / GPU / RAM tabs of the process list, "Add chart"), which are not readings. The "·" between the parts of a
 * chart's label line and lines of signs only (a menu's chevron) go too.
 */
export function tileCopyText(raw: string, skip: readonly string[] = []): string {
  const drop = new Set(skip.map((s) => s.replace(/\s+/g, " ").trim()).filter(Boolean));
  const out: string[] = [];
  for (const line of raw.split(/\r?\n/)) {
    const text = line
      .replace(/\s+/g, " ")
      .trim()
      .replace(/^(?:·\s*)+|(?:\s*·)+$/g, "");
    if (!/[\p{L}\p{N}]/u.test(text) || drop.has(text) || text === out[out.length - 1]) continue;
    out.push(text);
  }
  return out.join("\n");
}
