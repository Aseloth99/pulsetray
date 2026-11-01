<script lang="ts">
  /**
   * A line over calendar time (days to years), for the battery history: points can be far apart and unevenly
   * spaced, so the x axis is real time and the ends are labelled with dates. Hover reads a point out above the chart.
   * `shade`: stretches drawn behind the line (on AC power, for the charge level).
   */
  type Pt = { t: number; v: number; note?: string };
  let {
    points,
    color,
    max,
    min = 0,
    height = 90,
    fmt,
    dateFmt,
    shade = [],
    step = false,
  }: {
    points: Pt[];
    color: string;
    max?: number;
    min?: number;
    height?: number;
    fmt: (v: number) => string;
    dateFmt: (t: number) => string;
    shade?: { from: number; to: number }[];
    /** Hold each value until the next point (state changes) instead of a straight line between them. */
    step?: boolean;
  } = $props();

  const W = 1000;
  const H = 100;
  const t0 = $derived(points.length ? points[0].t : 0);
  const t1 = $derived(points.length ? points[points.length - 1].t : 1);
  const span = $derived(Math.max(1, t1 - t0));
  const top = $derived(max ?? Math.max(...points.map((p) => p.v), min + 1));
  const x = (t: number) => ((t - t0) / span) * W;
  const y = (v: number) => H - 2 - ((Math.min(Math.max(v, min), top) - min) / Math.max(1e-9, top - min)) * (H - 8);
  const line = $derived.by(() => {
    if (!points.length) return "";
    const out: string[] = [];
    points.forEach((p, i) => {
      if (step && i > 0) out.push(`${x(p.t).toFixed(1)},${y(points[i - 1].v).toFixed(1)}`);
      out.push(`${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`);
    });
    return out.join(" ");
  });
  const area = $derived(line ? `${x(t0).toFixed(1)},${H} ${line} ${x(t1).toFixed(1)},${H}` : "");

  let el = $state<HTMLElement>();
  let hoverX = $state<number | null>(null);
  const hovered = $derived.by(() => {
    if (hoverX === null || !points.length) return null;
    const target = t0 + hoverX * span;
    let best = points[0];
    for (const p of points) if (Math.abs(p.t - target) < Math.abs(best.t - target)) best = p;
    return best;
  });
  function onMove(e: PointerEvent) {
    if (!el) return;
    const r = el.getBoundingClientRect();
    hoverX = r.width ? Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)) : null;
  }
</script>

<svelte:window onblur={() => (hoverX = null)} />

<div class="chart">
  <div class="labels">
    <span>{fmt(min)} – {fmt(top)}</span>
    {#if hovered}
      <span class="read"><b>{fmt(hovered.v)}</b>{hovered.note ? ` · ${hovered.note}` : ""} · {dateFmt(hovered.t)}</span>
    {/if}
  </div>
  <div
    class="plot"
    style="height:{height}px"
    bind:this={el}
    onpointermove={onMove}
    onpointerleave={() => (hoverX = null)}
    role="presentation"
  >
    <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-hidden="true">
      {#each shade as s, i (i)}
        <rect class="shade" x={x(Math.max(s.from, t0))} width={Math.max(0, x(Math.min(s.to, t1)) - x(Math.max(s.from, t0)))} y="0" height={H} />
      {/each}
      {#each [0.25, 0.5, 0.75] as f (f)}
        <line class="grid" x1="0" x2={W} y1={y(min + (top - min) * f)} y2={y(min + (top - min) * f)} />
      {/each}
      {#if points.length > 1}
        <!-- with a shaded background the filled area would hide it: line only -->
        {#if !shade.length}<polygon points={area} fill={color} class="area" />{/if}
        <polyline points={line} stroke={color} class="line" />
      {/if}
    </svg>
    {#if hovered}
      <i class="cross" style="left:{(x(hovered.t) / W) * 100}%"></i>
      <i class="dot" style="left:{(x(hovered.t) / W) * 100}%;top:{(y(hovered.v) / H) * 100}%;border-color:{color}"></i>
    {/if}
  </div>
  <div class="axis">
    <span>{points.length ? dateFmt(t0) : ""}</span>
    <span>{points.length ? dateFmt(t1) : ""}</span>
  </div>
</div>

<style>
  .chart {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .labels,
  .axis {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    font-size: 10px;
    color: var(--muted);
    min-width: 0;
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
  .shade {
    fill: color-mix(in srgb, var(--bat) 20%, transparent);
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
    width: 7px;
    height: 7px;
    margin: -4px 0 0 -4px;
    border-radius: 50%;
    border: 2px solid;
    background: var(--bg);
    pointer-events: none;
  }
</style>
