import test from "node:test";
import assert from "node:assert/strict";
import { lib, read } from "./load.mjs";

const d = await lib("tiledetail");

test("the default charts are the first option, and none when that is 'none'", () => {
  assert.deepEqual(d.defaultCharts("cpu"), ["usage"]);
  assert.deepEqual(d.defaultCharts("memory"), ["ram"]);
  assert.deepEqual(d.defaultCharts("gpu"), ["usage"]);
  assert.deepEqual(d.defaultCharts("disk"), ["active"], "the disk tile starts with its activity chart");
  assert.deepEqual(d.defaultCharts("network"), [], "a tile without a chart choice has none");
  assert.deepEqual(d.defaultCharts("nope"), []);
});

test("cleanCharts keeps known ids once each, in the user's order, and drops 'none' and strangers", () => {
  assert.deepEqual(d.cleanCharts("cpu", ["power", "usage", "power", "none", "bogus", 7, null]), ["power", "usage"]);
  assert.deepEqual(d.cleanCharts("cpu", []), []);
  assert.deepEqual(d.cleanCharts("network", ["usage"]), [], "a tile without charts accepts none");
});

test("storableCharts stores nothing for the default and [] for 'no chart at all'", () => {
  assert.equal(d.storableCharts("cpu", ["usage"]), null);
  assert.deepEqual(d.storableCharts("cpu", []), [], "an empty list is a choice on a tile that has a default chart");
  assert.deepEqual(d.storableCharts("cpu", ["usage", "temp"]), ["usage", "temp"]);
  assert.deepEqual(d.storableCharts("cpu", ["temp", "usage"]), ["temp", "usage"], "order is part of the choice");
  assert.equal(d.storableCharts("disk", ["active"]), null);
  assert.deepEqual(d.storableCharts("disk", []), [], "no chart at all is a choice on the disk tile too");
  assert.deepEqual(d.storableCharts("disk", ["read", "write"]), ["read", "write"]);
});

test("add / remove / replace keep the order and never duplicate", () => {
  assert.deepEqual(d.addedChart(["usage"], "temp"), ["usage", "temp"]);
  assert.deepEqual(d.addedChart(["usage", "temp"], "usage"), ["usage", "temp"]);
  assert.deepEqual(d.removedChart(["usage", "temp", "power"], "temp"), ["usage", "power"]);
  assert.deepEqual(d.removedChart(["usage"], "gone"), ["usage"]);
  assert.deepEqual(d.replacedChart(["usage", "temp"], "usage", "power"), ["power", "temp"], "same slot");
  assert.deepEqual(d.replacedChart(["usage", "temp"], "usage", "temp"), ["temp"], "the target is on the list already");
  assert.deepEqual(d.replacedChart(["usage", "temp"], "usage", "usage"), ["usage", "temp"]);
  const input = ["usage"];
  d.addedChart(input, "temp");
  assert.deepEqual(input, ["usage"], "the input list is not changed");
});

test("saved settings: several charts, the old single `chart`, and rubbish", () => {
  const out = d.cleanDetails({
    cpu: { hide: [], badge: null, charts: ["temp", "usage", "usage", "bogus"] },
    gpu: { chart: "temp" }, // written before several charts were possible
    memory: { chart: "none" }, // "no chart" in the old format
    disk: { chart: "read" },
    wifi: { charts: ["usage"] }, // a tile without charts
    network: "nonsense",
  });
  assert.deepEqual(out.cpu.charts, ["temp", "usage"]);
  assert.deepEqual(out.gpu.charts, ["temp"]);
  assert.deepEqual(out.memory.charts, []);
  assert.deepEqual(out.disk.charts, ["read"]);
  assert.equal(out.wifi, undefined);
  assert.equal(out.network, undefined);
});

test("saved settings: the default is not stored, an empty list is", () => {
  const out = d.cleanDetails({
    cpu: { chart: "usage" }, // the default written the old way
    gpu: { charts: ["usage"] },
    memory: { charts: [] },
    disk: { charts: ["active"] },
  });
  assert.equal(out.cpu, undefined);
  assert.equal(out.gpu, undefined);
  assert.deepEqual(out.memory, { hide: [], badge: null, charts: [] });
  assert.equal(out.disk, undefined, "the disk default is its activity chart");
});

test("cleanDetails output survives being saved and read again", () => {
  const first = d.cleanDetails({ cpu: { hide: ["cache"], badge: "clock", charts: ["power", "usage"] }, disk: { charts: ["write"] } });
  const again = d.cleanDetails(JSON.parse(JSON.stringify(first)));
  assert.deepEqual(again, first);
});

test("every tile's chart ids are in the tiles that draw them", () => {
  // The tile components map each id of DETAIL to a series; an id nobody can draw would show an empty chart.
  const sources = {
    cpu: read("src/lib/tiles/CpuTile.svelte"),
    memory: read("src/lib/tiles/MemoryTile.svelte"),
    gpu: read("src/lib/tiles/GpuTile.svelte"),
    disk: read("src/lib/tiles/DiskTile.svelte"),
  };
  for (const [tile, src] of Object.entries(sources)) {
    assert.match(src, /<ChartStack tile="[a-z]+"/, `${tile} draws its charts with ChartStack`);
    for (const [id] of d.DETAIL[tile].charts) {
      if (id === "none") continue;
      assert.match(src, new RegExp(`\\b${id}:\\s*(\\{|def\\()`), `${tile} can draw "${id}"`);
    }
  }
});

test("a chart moves one place at a time and stays inside the list", () => {
  assert.deepEqual(d.movedChart(["usage", "temp", "power"], "power", -1), ["usage", "power", "temp"]);
  assert.deepEqual(d.movedChart(["usage", "temp", "power"], "usage", 1), ["temp", "usage", "power"]);
  assert.deepEqual(d.movedChart(["usage", "temp"], "usage", -1), ["usage", "temp"], "already first");
  assert.deepEqual(d.movedChart(["usage", "temp"], "temp", 1), ["usage", "temp"], "already last");
  assert.deepEqual(d.movedChart(["usage", "temp"], "gone", 1), ["usage", "temp"]);
});

test("a GPU only offers the charts it has a reading for", () => {
  const tile = read("src/lib/tiles/GpuTile.svelte");
  assert.match(tile, /has=\{canDraw\(gpu\)\}/);
  assert.match(tile, /id === "power" \? g\.sensors\.powerW != null/);
  const stack = read("src/lib/ChartStack.svelte");
  assert.match(stack, /caps\.chartAdd\(tile, has\)/);
  assert.match(stack, /caps\.chartPick\(tile, id, has\)/);
});

test("the scroll area is its own layer (smooth scrolling with the blurred shadows)", () => {
  assert.match(read("src/routes/+page.svelte"), /main \{[^}]*will-change: scroll-position/);
});

test("a long window's slots stay put as new samples arrive (the line must not shimmer every second)", async () => {
  const { slotSamples } = await lib("slots");
  const W = 1000;
  const windowSecs = 900; // 15 min: 900 one-second samples into 300 slots
  const value = (sec) => 20 + ((sec * 7919) % 61); // jumpy, deterministic
  const frame = (nowSec) => {
    const raw = [];
    for (let s = nowSec - windowSecs + 1; s <= nowSec; s++) {
      raw.push({ i: raw.length, x: (1 - (nowSec - s) / windowSecs) * W, v: value(s), t: s * 1000, gap: false });
    }
    return slotSamples(raw, { width: W, windowSecs, maxPoints: 300, gapMs: 3500, latest: nowSec * 1000 });
  };
  const a = frame(10_000);
  const b = frame(10_001);
  assert.ok(a.length <= 301 && b.length <= 301, "the path stays a few hundred points long");
  // Heights of the slots in the middle of the window are the same one second later (only the edges change).
  const byT = (pts) => new Map(pts.map((p) => [p.t, p.v]));
  const mb = byT(b);
  let same = 0;
  let total = 0;
  for (const p of a.slice(5, -5)) {
    total++;
    if (mb.get(p.t) === p.v) same++;
  }
  assert.ok(same / total > 0.99, `slot heights changed between frames: ${same}/${total} unchanged`);
  // And a slot moves left by exactly the time that passed, not by a re-dealt amount.
  const shift = b.find((p) => p.t === a[100].t)?.x - a[100].x;
  assert.ok(Math.abs(shift + W / windowSecs) < 1e-6, `slot x moved ${shift}`);
  // A short spike survives slotting.
  const raw = [];
  for (let s = 0; s < 900; s++) raw.push({ i: s, x: (s / 899) * W, v: s === 450 ? 100 : 10, t: s * 1000, gap: false });
  const out = slotSamples(raw, { width: W, windowSecs: 900, maxPoints: 300, gapMs: 3500, latest: 899_000 });
  assert.equal(Math.max(...out.map((p) => p.v)), 100);
  // Few samples are drawn as they are.
  assert.equal(slotSamples(raw.slice(0, 100), { width: W, windowSecs: 900, maxPoints: 300, gapMs: 3500, latest: 99_000 }).length, 100);
});
