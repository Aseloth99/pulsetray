import test from "node:test";
import assert from "node:assert/strict";
import { lib, read } from "./load.mjs";

const d = await lib("tiledetail");

test("the default charts are the first option, and none when that is 'none'", () => {
  assert.deepEqual(d.defaultCharts("cpu"), ["usage"]);
  assert.deepEqual(d.defaultCharts("memory"), ["ram"]);
  assert.deepEqual(d.defaultCharts("gpu"), ["usage"]);
  assert.deepEqual(d.defaultCharts("disk"), ["active"], "the disk tile starts with its activity chart");
  assert.deepEqual(d.defaultCharts("network"), ["down"], "the network tile starts with its download chart");
  assert.deepEqual(d.defaultCharts("wifi"), [], "a tile without a chart choice has none");
  assert.deepEqual(d.defaultCharts("nope"), []);
});

test("cleanCharts keeps known ids once each, in the user's order, and drops 'none' and strangers", () => {
  assert.deepEqual(d.cleanCharts("cpu", ["power", "usage", "power", "none", "bogus", 7, null]), ["power", "usage"]);
  assert.deepEqual(d.cleanCharts("cpu", []), []);
  assert.deepEqual(d.cleanCharts("wifi", ["usage"]), [], "a tile without charts accepts none");
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

test("a chart's label row has a fixed height (hovering must not change the tile height and resize the window)", () => {
  const body = read("src/lib/Spark.svelte").match(/\n  \.labels \{([^}]*)\}/)?.[1] ?? "";
  assert.match(body, /\n\s*height: [\d.]+(em|px);/, "the peak / hovered / mirrored readings differ by a fraction of a pixel");
});

test("the popup is moved and resized in one step (a move, then a resize, showed a frame in between)", () => {
  const rs = read("src-tauri/src/lib.rs");
  const fn = (name) => rs.match(new RegExp(`\\nfn ${name}\\([\\s\\S]*?\\n\\}\\n`))?.[0] ?? "";
  assert.match(fn("move_and_resize"), /SetWindowPos\(hwnd, None, pos\.x, pos\.y, w, h, flags\)/);
  assert.match(fn("place_popup"), /move_and_resize\(/);
  assert.doesNotMatch(fn("place_popup"), /set_size\(/, "the fitted size goes out with the move");
  assert.match(fn("resize_popup_to"), /move_and_resize\(/);
  assert.doesNotMatch(fn("resize_popup_to"), /set_position\(/, "the new position goes out with the new size");
});

test("the update screen hugs its content even in a popup with a size of the user's own, and gives that size back", () => {
  const page = read("src/routes/+page.svelte");
  const rs = read("src-tauri/src/lib.rs");
  const fn = (name) => rs.match(new RegExp(`\\nfn ${name}\\([\\s\\S]*?\\n\\}\\n`))?.[0] ?? "";
  // a pinned or dragged popup ignores set_popup_height: the short screen sat at the top of a big, empty window
  assert.match(page.match(/function fitUpdate\(\) \{[\s\S]*?\n  \}/)?.[0] ?? "", /invoke\("fit_update_view"/);
  assert.match(page, /if \(onUpdateView && !now\) invoke\("leave_update_view"\)/);
  assert.match(page.match(/function saveUserSize\(\) \{[\s\S]*?\n  \}/)?.[0] ?? "", /if \(view === "update"\) return;/);
  assert.match(fn("fit_update_view"), /if !size_is_fixed\(&state\) \{\s*set_popup_height\(/, "an auto-fitting popup fits as before");
  assert.match(fn("fit_update_view"), /get_or_insert\(now\)/, "only the first fit puts the user's size aside");
  assert.match(fn("leave_update_view"), /\.take\(\)/);
  assert.match(fn("show_popup"), /update_view_saved\.lock\(\)\.unwrap\(\)\.take\(\)[\s\S]*?w\.set_size\(s\)/, "hidden on the update screen");
  for (const name of ["fit_update_view", "leave_update_view"]) assert.match(rs, new RegExp(`\\n\\s+${name},\\n`), `${name} is registered`);
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

test("cpu: the package power can be the badge; unreadable, it is greyed out like the power row and chart", () => {
  const cpu = d.DETAIL.cpu;
  assert.equal(cpu.badges[0][0], "temp", "the temperature stays the default");
  assert.ok(cpu.badges.some(([id]) => id === "power"));
  assert.equal(d.cleanDetails({ cpu: { hide: [], badge: "power" } }).cpu.badge, "power", "the choice is kept");
  assert.match(read("src/lib/caps.svelte.ts"), /live\("cpu\.power", m\.cpuPowerW != null, "capCpuPower"\);/);
  const tile = read("src/lib/tiles/CpuTile.svelte");
  assert.match(tile, /badgeKind === "power" && m\.cpuPowerW != null\s*\? wattsNum\(m\.cpuPowerW\)/);
});

test("network: download by default, upload and the total a click away, the live rates in the badge", () => {
  const net = d.DETAIL.network;
  assert.deepEqual(net.charts.map(([id]) => id), ["down", "up", "total", "none"]);
  assert.equal(net.badges[0][0], "rates", "the badge shows download and upload by default");
  assert.ok(net.badges.some(([id]) => id === "total") && net.badges.some(([id]) => id === "today"));
  assert.ok(!net.rows.some(([id]) => id === "downChart" || id === "upChart"), "the charts are no longer rows");
});

test("network: an old setting that hid one of the two chart rows keeps it hidden", () => {
  assert.equal(d.cleanDetails({ network: { hide: ["upChart"], badge: null } }).network, undefined, "download alone is the default");
  assert.deepEqual(d.cleanDetails({ network: { hide: ["downChart"], badge: null } }).network.charts, ["up"]);
  assert.deepEqual(d.cleanDetails({ network: { hide: ["downChart", "upChart"], badge: null } }).network.charts, []);
  assert.equal(d.cleanDetails({ network: { hide: ["ip"], badge: null } }).network.charts, null, "nothing hidden: the new default");
  assert.deepEqual(d.cleanDetails({ network: { hide: ["upChart", "ip"], badge: null } }).network.hide, ["ip"], "old rows dropped");
});

test("network: the total rate is a chart series, live and from the backend's history", () => {
  assert.match(read("src/lib/metrics.svelte.ts"), /put\("net\.total", m\.netDown \+ m\.netUp\);/);
  assert.match(read("src-tauri/src/history.rs"), /col\("net\.total", &\|p\| Some\(\(p\.down \+ p\.up\)\.round\(\)\)\);/);
  const tile = read("src/lib/tiles/NetworkTile.svelte");
  assert.match(tile, /<ChartStack tile="network" defs=\{CHARTS\}/);
  assert.match(tile, /badgePick=\{adv \? caps\.quick\("network", "badge"\)/);
});
