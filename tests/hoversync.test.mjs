import test from "node:test";
import assert from "node:assert/strict";
import { lib, read } from "./load.mjs";

const { nearestIndex, xOfTime } = await lib("nearest");

const pts = (xs, nulls = []) => xs.map((x, i) => ({ i, x, v: nulls.includes(i) ? null : i * 10, t: 1000 + i, gap: false }));

/** The plain scan Spark used before (the reference). */
function scan(list, target, maxDist) {
  let best = -1;
  let dist = Infinity;
  list.forEach((p, i) => {
    if (p.v == null) return;
    const d = Math.abs(p.x - target);
    if (d < dist) {
      dist = d;
      best = i;
    }
  });
  return best >= 0 && dist <= maxDist ? best : -1;
}

test("nearest sample: the same answer as the plain scan, also past gaps and at the edges", () => {
  const xs = [0, 10, 20, 30, 45, 60, 61, 80, 100];
  for (const nulls of [[], [3, 4], [0, 8], [5, 6, 7], [0, 1, 2, 3, 4, 5, 6, 7, 8]]) {
    const list = pts(xs, nulls);
    for (let target = -20; target <= 120; target += 0.5) {
      for (const maxDist of [3, 12, 40, Infinity]) {
        assert.equal(nearestIndex(list, target, maxDist), scan(list, target, maxDist), `t=${target} max=${maxDist} nulls=${nulls}`);
      }
    }
  }
});

test("nearest sample: of two equally near ones the earlier wins, an empty chart has none", () => {
  assert.equal(nearestIndex(pts([0, 10]), 5, 10), 0);
  assert.equal(nearestIndex([], 5, 10), -1);
});

test("a moment's place on a chart: newest at the right edge, the window's start at the left", () => {
  assert.equal(xOfTime(100_000, 100_000, 60, 1000), 1000);
  assert.equal(xOfTime(40_000, 100_000, 60, 1000), 0);
  assert.equal(xOfTime(70_000, 100_000, 60, 1000), 500);
});

test("charts mirror the hovered moment: the owner publishes it, the others read it (dashed line, own reading)", () => {
  const spark = read("src/lib/Spark.svelte");
  const sync = read("src/lib/hoversync.svelte.ts");
  assert.match(spark, /publishHover\(me, hovered\?\.t \?\? null\)/);
  assert.match(spark, /hoverSync\.owner === me/, "the hovered chart doesn't mirror itself");
  assert.match(spark, /class="cross sync"/);
  assert.match(spark, /\{:else if synced\}\s*<span class="read"><b>\{fmt\(synced\.v as number\)\}<\/b>/);
  assert.match(sync, /else if \(hoverSync\.owner === owner\)/, "only the owner clears it");
  // the tooltip and the "who used the most" question stay with the chart under the pointer
  assert.doesNotMatch(spark, /const h = synced/);
});

test("the chart that published the moment can clear it again (compiled with real runes: the owner is not a state proxy)", async () => {
  // A deep $state would keep a proxy of the chart's object: `===` never matched, the moment was never cleared and every
  // chart kept its dashed line after the pointer left (up to 1.0.9). Compiled here exactly as the app gets it.
  const { compileModule } = await import("svelte/compiler");
  const { stripTypeScriptTypes } = await import("node:module");
  const src = stripTypeScriptTypes(read("src/lib/hoversync.svelte.ts"));
  let { code } = compileModule(src, { generate: "client", filename: "hoversync.svelte.js" }).js;
  code = code.replace(/(from\s+)(["'])(svelte\/[^"']+)\2/g, (_, from, q, spec) => `${from}${q}${import.meta.resolve(spec)}${q}`);
  const { hoverSync, publishHover } = await import(`data:text/javascript,${encodeURIComponent(code)}`);

  const a = {};
  const b = {};
  publishHover(a, 1000);
  assert.equal(hoverSync.owner, a, "the owner is the chart's own object");
  assert.equal(hoverSync.t, 1000);
  publishHover(b, 2000); // the pointer moved straight onto another chart
  publishHover(a, null); // the first chart's leave comes after: it no longer owns the moment
  assert.equal(hoverSync.owner, b);
  assert.equal(hoverSync.t, 2000);
  publishHover(b, null); // the pointer left the charts
  assert.equal(hoverSync.owner, null);
  assert.equal(hoverSync.t, null);
});
