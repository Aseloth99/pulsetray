<script lang="ts">
  /** Hours per day, two stacked parts (on battery, on AC); hover reads a day out above the bars. */
  type Day = { label: string; a: number; b: number };
  let {
    days,
    colorA,
    colorB,
    labelA,
    labelB,
    height = 90,
    fmt,
  }: {
    days: Day[];
    colorA: string;
    colorB: string;
    labelA: string;
    labelB: string;
    height?: number;
    fmt: (hours: number) => string;
  } = $props();

  const top = $derived(Math.max(1, ...days.map((d) => d.a + d.b)));
  let hover = $state<number | null>(null);
  const h = $derived(hover === null ? null : days[hover]);
</script>

<div class="chart">
  <div class="labels">
    <span class="legend"><i style="background:{colorA}"></i>{labelA}<i style="background:{colorB}"></i>{labelB}</span>
    {#if h}
      <span class="read">{h.label} · <b style="color:{colorA}">{fmt(h.a)}</b> · <b style="color:{colorB}">{fmt(h.b)}</b></span>
    {/if}
  </div>
  <div class="bars" style="height:{height}px" role="presentation" onpointerleave={() => (hover = null)}>
    {#each days as d, i (i)}
      {@const sum = d.a + d.b || 1}
      <div class="day" class:on={hover === i} role="presentation" onpointerenter={() => (hover = i)}>
        <!-- one line, fixed shares: no whitespace or flex rounding between the two parts -->
        <span class="daybar" style="height:{((d.a + d.b) / top) * 100}%"
          ><i style="height:{(d.b / sum) * 100}%;background:{colorB}"></i><i style="height:{(d.a / sum) * 100}%;background:{colorA}"></i
        ></span>
      </div>
    {/each}
  </div>
  <div class="axis">
    <span>{days.length ? days[0].label : ""}</span>
    <span>{days.length ? days[days.length - 1].label : ""}</span>
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
  .legend {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  .legend i {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    margin-left: 4px;
  }
  .legend i:first-child {
    margin-left: 0;
  }
  .read {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .read b {
    font-weight: 650;
  }
  .bars {
    display: flex;
    align-items: stretch;
    gap: 3px;
    padding: 4px;
    border-radius: 8px;
    box-shadow: var(--inset-sm);
  }
  .day {
    flex: 1;
    display: flex;
    align-items: flex-end;
    border-radius: 4px;
    min-width: 0;
  }
  .day.on {
    background: color-mix(in srgb, var(--muted) 14%, transparent);
  }
  .daybar {
    width: 100%;
    display: flex;
    flex-direction: column;
    border-radius: 3px;
    overflow: hidden;
    min-height: 0;
  }
  .daybar i {
    display: block;
    flex: none;
  }
</style>
