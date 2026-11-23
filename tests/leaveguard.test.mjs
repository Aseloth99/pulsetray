import test from "node:test";
import assert from "node:assert/strict";
import { lib, read } from "./load.mjs";

const { LeaveGuard, pointInside } = await lib("leaveguard");

/** A guard whose probe answers from a list (true: the pointer is still over the window). */
function guard(answers) {
  const calls = { n: 0 };
  const g = new LeaveGuard(async () => {
    calls.n++;
    const a = answers.shift();
    if (a instanceof Error) throw a;
    return a;
  }, 60_000);
  return { g, calls };
}

test("leave guard: a hover is cleared once the pointer is outside the window, not while it is inside", async () => {
  const { g } = guard([true, true, false]);
  let left = 0;
  const stop = g.watch(() => left++);
  await g.check();
  await g.check();
  assert.equal(left, 0);
  await g.check();
  assert.equal(left, 1);
  assert.equal(g.watching, 0);
  stop(); // cancelling after it fired is harmless
  await g.check();
  assert.equal(left, 1);
});

test("leave guard: every waiting hover is cleared, a cancelled one is not; nothing is asked while nothing waits", async () => {
  const { g, calls } = guard([false]);
  const seen = [];
  g.watch(() => seen.push("chart"));
  const stopTip = g.watch(() => seen.push("tooltip"));
  g.watch(() => seen.push("cores"));
  stopTip();
  await g.check();
  assert.deepEqual(seen, ["chart", "cores"]);
  await g.check();
  assert.equal(calls.n, 1);
});

test("leave guard: where the position cannot be asked (site preview) it steps aside and the DOM events decide", async () => {
  for (const answer of [null, new Error("no Tauri")]) {
    const { g, calls } = guard([answer]);
    let left = 0;
    g.watch(() => left++);
    await g.check();
    assert.equal(left, 0, "a hover is not cleared on a failed reading");
    assert.equal(g.watching, 0);
    g.watch(() => left++);
    assert.equal(g.watching, 0, "no more polling once unsupported");
    await g.check();
    assert.equal(calls.n, 1);
  }
});

test("leave guard: the pointer inside the window rectangle, edges and a bad reading", () => {
  const pos = { x: -1920, y: 100 }; // a window on a monitor left of the primary one
  const size = { width: 400, height: 600 };
  assert.equal(pointInside({ x: -1920, y: 100 }, pos, size), true);
  assert.equal(pointInside({ x: -1521, y: 699 }, pos, size), true);
  assert.equal(pointInside({ x: -1520, y: 300 }, pos, size), false);
  assert.equal(pointInside({ x: -1700, y: 99 }, pos, size), false);
  assert.equal(pointInside({ x: 10, y: 10 }, pos, size), false);
  assert.equal(pointInside({ x: undefined, y: 10 }, pos, size), null);
  assert.equal(pointInside({ x: 1, y: 1 }, pos, { width: 0, height: 0 }), null);
});

test("leave guard: every hover state of the charts, the CPU cores and the tooltip is watched", () => {
  for (const [file, state] of [
    ["src/lib/Spark.svelte", "hoverX = null"],
    ["src/lib/DateChart.svelte", "hoverX = null"],
    ["src/lib/UsageBars.svelte", "hover = null"],
    ["src/lib/tiles/CpuTile.svelte", "coreIdx = null"],
  ]) {
    assert.match(read(file), new RegExp(`watchLeave\\(\\(\\) => \\(${state.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\)\\)`), file);
  }
  assert.match(read("src/lib/tipstate.svelte.ts"), /watchLeave\(/);
});
