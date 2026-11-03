<script module lang="ts">
  const STEPS = [1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10];

  /**
   * A round chart ceiling just above `v`: 1, 1.2, 1.5, 2 ... 8, 10 times a power of ten. Byte values are rounded
   * inside their unit (KB, MB, ...), so the scale reads "60 MB/s", not "62914560".
   */
  export function niceTop(v: number, bytes = false): number {
    if (!(v > 0)) return 1;
    let unit = 1;
    if (bytes) while (v / unit >= 1024) unit *= 1024;
    const m = v / unit;
    const d = 10 ** Math.floor(Math.log10(m));
    const s = STEPS.find((x) => m / d <= x + 1e-9) ?? 10;
    const top = s * d;
    return bytes && top >= 1000 ? 1024 * unit : top * unit;
  }

  /** More samples than this in the window (15 min, 1 h) are drawn as this many time slots, each its highest value. */
  export const MAX_POINTS = 300;
</script>

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { tr } from "./tr";
  import type { Key } from "./i18n";
  import { bytes as fmtBytes, rate } from "./format";
  import { tooltip, tip } from "./tipstate.svelte";
  import Icon from "./Icon.svelte";
  import OptionMenu from "./OptionMenu.svelte";
  import type { QuickPick } from "./tiledetail";

  /** "Who caused this peak?": the apps that used the most of this resource around the hovered moment. */
  type Who = "cpu" | "mem" | "disk" | "gpu";
  type Tops = { from: number; to: number; apps: { name: string; value: number }[] } | null;
  const WHO_TITLE: Record<Who, Key> = { cpu: "whoCpu", mem: "whoMem", disk: "whoDisk", gpu: "whoGpu" };
  const share = (v: number) => (v < 10 ? `${v.toFixed(1)}%` : `${v.toFixed(0)}%`);
  const WHO_FMT: Record<Who, (v: number) => string> = { cpu: share, gpu: share, mem: (v) => fmtBytes(v), disk: rate };

  type V = number | null;
  let {
    data,
    times,
    windowSecs = 60,
    color,
    max,
    height = 26,
    fmt,
    bytes = false,
    floor = 1,
    detail = false,
    gapMs = 3500,
    label,
    pick,
    who,
  }: {
    data: V[];
    /** When each sample arrived (ms); `data` is aligned to the end of it. Without it samples are spread evenly. */
    times?: number[];
    /** How much time the chart shows (with `times`). */
    windowSecs?: number;
    color: string;
    /** Fixed ceiling (100 for percentages); otherwise a round number above the highest visible value. */
    max?: number;
    height?: number;
    /** Value formatter: enables the hover readout (and the labels in `detail`). */
    fmt?: (v: number) => string;
    /** Values are bytes: the automatic ceiling is rounded within KB / MB / GB. */
    bytes?: boolean;
    /** Lowest automatic ceiling (an idle line shouldn't fill the whole height). */
    floor?: number;
    /** Advanced view: grid lines, the scale and the peak above the chart. */
    detail?: boolean;
    /** A longer pause between samples (the popup was closed) leaves the stretch empty instead of drawing a bridge. */
    gapMs?: number;
    /** What the series is ("RAM", "Sanal bellek", "Sıcaklık"...): shown before the scale in `detail`, in the series
     *  colour — a tile that can chart several things otherwise said only "100%". */
    label?: string;
    /** Makes the label a button that lists the other things this tile can chart. */
    pick?: QuickPick;
    /** Charts with a time axis: the hover also names the apps that used the most then (CPU, memory, disk, GPU). */
    who?: Who;
  } = $props();

  // viewBox units; the svg is stretched to its box while strokes keep their width (non-scaling-stroke).
  const W = 1000;
  const H = 100;

  /** `gap`: the line breaks before this point (the popup was closed, the PC slept). */
  type P = { i: number; x: number; v: V; t: number | null; gap: boolean };
  const latest = $derived(times && times.length ? times[times.length - 1] : 0);

  /** Every sample inside the window, at its place on the time axis. */
  const raw = $derived.by(() => {
    const n = data.length;
    const out: P[] = [];
    if (times && times.length >= n && n) {
      const off = times.length - n;
      const span = windowSecs * 1000;
      let lastT: number | null = null;
      for (let i = 0; i < n; i++) {
        const t = times[off + i];
        const age = latest - t;
        if (age > span) continue;
        out.push({ i, x: (1 - age / span) * W, v: data[i], t, gap: lastT != null && t - lastT > gapMs });
        lastT = t;
      }
    } else {
      for (let i = 0; i < n; i++) out.push({ i, x: n > 1 ? (i / (n - 1)) * W : W, v: data[i], t: null, gap: false });
    }
    return out;
  });

  /**
   * What is drawn: the samples themselves, or for a long window one point per time slot with the slot's highest
   * value (a short spike stays visible, the path stays a few hundred points long however long the window is).
   */
  const pts = $derived.by(() => {
    if (raw.length <= MAX_POINTS || raw[0].t == null) return raw;
    const slot = (windowSecs * 1000) / MAX_POINTS;
    const out: P[] = [];
    let cur: P | null = null;
    let curSlot = -1;
    let lastT: number | null = null;
    for (const p of raw) {
      const s = Math.min(MAX_POINTS - 1, Math.floor(((p.x / W) * windowSecs * 1000) / slot));
      if (s !== curSlot || p.gap) {
        if (cur) out.push(cur);
        cur = { ...p, x: ((s + 0.5) / MAX_POINTS) * W, gap: lastT != null && (p.t as number) - lastT > gapMs };
        curSlot = s;
      } else if (p.v != null && (cur!.v == null || p.v >= (cur!.v as number))) {
        cur = { ...cur!, v: p.v, t: p.t, i: p.i };
      }
      lastT = p.t;
    }
    if (cur) out.push(cur);
    return out;
  });

  /** The highest sample in the window (exact, also when the drawing is slotted). */
  const peak = $derived.by(() => {
    let best: P | null = null;
    // ">=": of equal highs the newest one (a flat line shouldn't point at its oldest sample).
    for (const p of raw) if (p.v != null && (best === null || p.v >= (best.v as number))) best = p;
    return best;
  });
  const top = $derived(max ?? niceTop(Math.max((peak?.v as number) ?? 0, floor) * 1.08, bytes));
  const y = (v: number) => H - 1 - (Math.min(Math.max(v, 0), top) / top) * (H - 6);

  /** Unbroken stretches of samples (a missing value or a pause breaks the line). */
  const runs = $derived.by(() => {
    const out: { x: number; y: number }[][] = [];
    let cur: { x: number; y: number }[] = [];
    for (const p of pts) {
      if ((p.v == null || p.gap) && cur.length) {
        out.push(cur);
        cur = [];
      }
      if (p.v != null) cur.push({ x: p.x, y: y(p.v) });
    }
    if (cur.length) out.push(cur);
    return out;
  });
  const line = (r: { x: number; y: number }[]) => r.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ");
  const area = (r: { x: number; y: number }[]) => `${r[0].x.toFixed(1)},${H} ${line(r)} ${r[r.length - 1].x.toFixed(1)},${H}`;

  // ---- hover: the sample under the pointer (computed only while the pointer is over the chart)
  let el = $state<HTMLElement>();
  let hoverX = $state<number | null>(null);
  const hovered = $derived.by(() => {
    if (hoverX === null || !fmt) return null;
    const target = hoverX * W;
    let best: P | null = null;
    let dist = Infinity;
    for (const p of pts) {
      if (p.v == null) continue;
      const d = Math.abs(p.x - target);
      if (d < dist) {
        dist = d;
        best = p;
      }
    }
    // Over an empty stretch (the popup was closed then) there is nothing to read.
    return best && dist <= W * 0.04 ? best : null;
  });

  function ago(p: P): string {
    const secs = p.t != null ? Math.round((latest - p.t) / 1000) : data.length - 1 - p.i;
    if (secs <= 0) return tr("now");
    if (secs < 60) return tr("agoS").replace("{n}", String(secs));
    return tr("agoM")
      .replace("{m}", String(Math.floor(secs / 60)))
      .replace("{s}", String(secs % 60));
  }
  const readout = (p: P) => (fmt ? `${fmt(p.v as number)} · ${ago(p)}` : "");
  const scale = $derived(fmt ? fmt(top).replace(/\.0+(?=\D|$)/, "") : "");

  // The backend keeps who used what for the whole chart history (history.rs); asked for the hovered moment only,
  // a moment after the pointer stops. The previous answer stays up meanwhile (no flicker while moving).
  let tops = $state<Tops>(null);
  let topsFor = "";
  $effect(() => {
    const h = hovered;
    const kind = who;
    if (!kind || !h || h.t == null) {
      tops = null;
      topsFor = "";
      return;
    }
    const key = `${kind}:${h.t}`;
    if (key === topsFor) return;
    const t = h.t;
    const timer = setTimeout(() => {
      topsFor = key;
      invoke<Tops>("top_apps_at", { t, kind })
        .then((r) => {
          if (topsFor === key) tops = r;
        })
        .catch(() => {});
    }, 60);
    return () => clearTimeout(timer);
  });
  const whoLines = $derived.by(() => {
    if (!who || !tops || !tops.apps.length) return [];
    const f = WHO_FMT[who];
    return [tr(WHO_TITLE[who]), ...tops.apps.map((a) => `${a.name.replace(/\.exe$/i, "")} · ${f(a.value)}`)];
  });

  // Small charts have no label row: the readout goes into the shared tooltip, just above the chart. Big ones
  // show it in their label row, and the tooltip (below the chart) only carries the apps.
  let ownsTip = false;
  $effect(() => {
    const h = hovered;
    const lines = whoLines;
    if (!el) return;
    if (!h || (detail && !lines.length)) {
      if (ownsTip) tooltip.hide();
      ownsTip = false;
      return;
    }
    const r = el.getBoundingClientRect();
    const text = detail ? lines.join("\n") : [readout(h), ...lines].join("\n");
    tooltip.show(text, new DOMRect(r.left + (h.x / W) * r.width, r.top, 0, r.height), !detail);
    ownsTip = true;
  });
  $effect(() => () => {
    if (ownsTip) tooltip.hide();
  });

  function onMove(e: PointerEvent) {
    if (!fmt || !el) return;
    const r = el.getBoundingClientRect();
    hoverX = r.width ? Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)) : null;
  }
</script>

<!-- A popup hidden while the pointer was on the chart must not come back with a stuck readout. -->
<svelte:window onblur={() => (hoverX = null)} />
<svelte:document onvisibilitychange={() => (hoverX = null)} />

<div class="chart">
  {#if detail && fmt}
    <div class="labels">
      <span class="scale">{#if label}{#if pick}<OptionMenu {pick} align="start">{#snippet trigger({ toggle, open })}<button
              type="button"
              class="name pick"
              style="color:{color}"
              aria-haspopup="menu"
              aria-expanded={open}
              use:tip={open ? "" : pick.tip}
              onclick={toggle}><b>{label}</b><Icon name="down" size={9} /></button
            >{/snippet}</OptionMenu>{:else}<b class="name" style="color:{color}">{label}</b>{/if}<span class="sep">·</span>{/if}{scale}</span>
      {#if hovered}
        <span class="read"><b>{fmt(hovered.v as number)}</b> · {ago(hovered)}</span>
      {:else if peak && (peak.v as number) > 0}
        <span class="read">{tr("peak")} <b>{fmt(peak.v as number)}</b> · {ago(peak)}</span>
      {/if}
    </div>
  {/if}
  <div
    class="plot"
    class:hover={fmt}
    style="height:{height}px"
    bind:this={el}
    onpointermove={onMove}
    onpointerleave={() => (hoverX = null)}
    role="presentation"
  >
    <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-hidden="true">
      {#if detail}
        {#each [0.25, 0.5, 0.75] as f (f)}
          <line class="grid" x1="0" x2={W} y1={y(top * f)} y2={y(top * f)} />
        {/each}
      {/if}
      {#each runs as r, k (k)}
        {#if r.length > 1}
          <polygon points={area(r)} fill={color} class="area" />
          <polyline points={line(r)} stroke={color} class="line" />
        {/if}
      {/each}
    </svg>
    {#if hovered}
      <i class="cross" style="left:{(hovered.x / W) * 100}%"></i>
      <i class="dot" style="left:{(hovered.x / W) * 100}%;top:{(y(hovered.v as number) / H) * 100}%;border-color:{color}"></i>
    {:else if detail && peak && (peak.v as number) > 0}
      <i class="dot peak" style="left:{(peak.x / W) * 100}%;top:{(y(peak.v as number) / H) * 100}%;background:{color}"></i>
    {/if}
  </div>
</div>

<style>
  .chart {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .labels {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
    font-size: 10px;
    color: var(--muted);
    min-width: 0;
  }
  .scale {
    flex: none;
    display: inline-flex;
    align-items: baseline;
    gap: 4px;
  }
  .name {
    font-weight: 700;
  }
  button.pick {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    border: 0;
    background: transparent;
    font: inherit;
    margin: -2px 0 -2px -6px;
    padding: 2px 5px 2px 6px;
    border-radius: 7px;
    cursor: pointer;
  }
  button.pick :global(svg) {
    opacity: 0.7;
  }
  button.pick:hover,
  button.pick[aria-expanded="true"] {
    box-shadow: var(--inset-sm);
  }
  .read {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .read b {
    color: var(--text);
    font-weight: 650;
  }
  .plot {
    position: relative;
    border-radius: 8px;
    background: var(--bg);
    box-shadow: var(--inset-sm);
    overflow: hidden;
  }
  .plot.hover {
    cursor: crosshair;
  }
  svg {
    display: block;
    width: 100%;
    height: 100%;
  }
  .grid {
    stroke: color-mix(in srgb, var(--muted) 22%, transparent);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .area {
    fill-opacity: 0.2;
    stroke: none;
  }
  .line {
    fill: none;
    stroke-width: 1.8;
    stroke-linejoin: round;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }
  .cross {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: color-mix(in srgb, var(--muted) 70%, transparent);
    pointer-events: none;
  }
  .dot {
    position: absolute;
    width: 8px;
    height: 8px;
    margin: -4px 0 0 -4px;
    border-radius: 50%;
    border: 2px solid;
    background: var(--bg);
    pointer-events: none;
  }
  .dot.peak {
    width: 6px;
    height: 6px;
    margin: -3px 0 0 -3px;
    border: 0;
  }
</style>
