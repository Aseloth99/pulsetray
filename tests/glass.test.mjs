import test from "node:test";
import assert from "node:assert/strict";
import { read } from "./load.mjs";

/** The rules of glass.css as [selector list, body] pairs (comments removed; the file has no nested at-rules). */
function rules() {
  const css = read("src/glass.css").replace(/\/\*[\s\S]*?\*\//g, "");
  return [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((m) => [m[1].trim(), m[2]]);
}

test("glass: only the floating panels blur what is behind them; nothing that scrolls does", () => {
  // A backdrop-filter on the tiles or cards made the web view redo a blur pass for each of them on every scrolled
  // frame (12.7 ms a frame instead of 1.6 ms), and wheel scrolling stuttered even at 144 Hz.
  const blurred = rules()
    .filter(([, body]) => /(^|[\s;])(-webkit-)?backdrop-filter\s*:/.test(body))
    .flatMap(([sel]) => sel.split(",").map((s) => s.trim()));
  assert.ok(blurred.length > 0, "the floating panels are still frosted");
  for (const sel of blurred) {
    assert.match(sel, /\.(tip|ctx|menu)\b/, `${sel} has a backdrop-filter`);
    assert.doesNotMatch(sel, /data-tile|\.card\b|\bmain\b/, `${sel} has a backdrop-filter`);
  }
});

test("glass: the tiles and cards keep their translucent fill", () => {
  const panel = rules().find(([sel]) => sel.includes("[data-tile] > section") && sel.includes(".shell .card"));
  assert.ok(panel, "the tile and card rule exists");
  assert.match(panel[1], /background:\s*var\(--g-card\)/);
});
