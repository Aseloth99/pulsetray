<script lang="ts">
  import Tile from "../Tile.svelte";
  import Donut from "../Donut.svelte";
  import Spark from "../Spark.svelte";
  import Bar from "../Bar.svelte";
  import Kv from "../Kv.svelte";
  import ChartStack, { type ChartDef } from "../ChartStack.svelte";
  import { tr } from "../tr";
  import { bytes, pct, pairBytes, shortName, percent, celsius, wattsNum } from "../format";
  import { store, primaryGpu, type Metrics, type Gpu } from "../metrics.svelte";
  import { settings } from "../settings.svelte";
  import { caps } from "../caps.svelte";

  let { m, adv, span }: { m: Metrics; adv: boolean; span: number } = $props();
  const GIB = 1024 ** 3;

  // Several GPUs (iGPU + dGPU, or two identical cards): one tab each, like the top-apps tile.
  let chosen = $state<string | null>(null);
  const gpu = $derived(m.gpus.find((g) => g.luid === chosen) ?? primaryGpu(m.gpus));

  /** Short tab names; identical cards get a number so they can be told apart. */
  const labels = $derived.by(() => {
    const names = m.gpus.map((g) => shortName(g.name));
    return m.gpus.map((g, i) => {
      const same = names.filter((n) => n === names[i]).length > 1;
      return same ? `${names[i]} #${names.slice(0, i + 1).filter((n) => n === names[i]).length}` : names[i];
    });
  });

  /** Integrated GPUs have a tiny dedicated pool and live in shared system memory instead. */
  function mem(g: Gpu) {
    return g.vramTotal >= GIB
      ? { label: tr("vram"), used: g.vramUsed, total: g.vramTotal }
      : { label: tr("sharedMem"), used: g.sharedUsed, total: g.sharedTotal };
  }

  const mhz = (v: number) => (v >= 1000 ? `${(v / 1000).toFixed(2)} GHz` : `${v} MHz`);

  const show = (row: string) => settings.shown("gpu", row);
  const badgeKind = $derived(caps.badge("gpu"));
  /** Header badge in the advanced view; the temperature (default) falls back to the load when there is no sensor. */
  function badgeOf(g: Gpu): { text?: string; hot: boolean } {
    const s = g.sensors;
    if (badgeKind === "none") return { hot: false };
    if (badgeKind === "power" && s.powerW != null) return { text: wattsNum(s.powerW), hot: false };
    if (badgeKind === "vram") {
      const mm = mem(g);
      return { text: pairBytes(mm.used, mm.total), hot: false };
    }
    if (badgeKind === "temp" && s.tempC != null) return { text: celsius(s.tempC), hot: true };
    return { text: percent(g.usage), hot: false };
  }
  /** What the advanced charts can draw; the power chart's top is the card's own limit. */
  const chartDefs = (g: Gpu): Record<string, ChartDef> => {
    const def = (id: string, fmt: (v: number) => string, label: string, max?: number): ChartDef => ({
      series: `gpu.${g.luid}.${id}`,
      fmt,
      max,
      label,
      color: "var(--gpu)",
    });
    return {
      usage: def("usage", percent, tr("usage"), 100),
      temp: def("temp", celsius, tr("temperature")),
      vram: def("vram", percent, tr("vram"), 100),
      power: def("power", wattsNum, tr("power"), g.sensors.powerLimitW || undefined),
    };
  };
</script>

{#if gpu}
  {@const s = gpu.sensors}
  {@const mm = mem(gpu)}
  {@const hot = s.tempC}
  {@const b = badgeOf(gpu)}
  <Tile
    title={tr("gpu")}
    icon="gpu"
    color="var(--gpu)"
    {span}
    badge={adv ? b.text : undefined}
    badgePick={caps.quick("gpu", "badge")}
    badgeTone={!b.hot || hot == null ? "neutral" : hot < 80 ? "ok" : hot < 90 ? "warn" : "bad"}
  >
    {#if m.gpus.length > 1}
      <div class="seg">
        {#each m.gpus as g, i (g.luid)}
          <button class:on={g.luid === gpu.luid} title={g.name} onclick={() => (chosen = g.luid)}>
            {labels[i]}
          </button>
        {/each}
      </div>
    {/if}

    {#if adv}
      {#if show("model")}<p class="model" title={gpu.name}>{gpu.name}</p>{/if}
      <div class="split">
        <div class="vis"><Donut value={gpu.usage} color="var(--gpu)" size={96} /></div>
        <div class="kv">
          {#if show("usage")}<Kv label={tr("usage")} value={percent(gpu.usage)} />{/if}
          {#if s.tempC != null && show("temp")}<Kv label={tr("temperature")} value={celsius(s.tempC)} />{/if}
          {#if s.powerW != null && show("power")}
            <Kv label={tr("power")} value={s.powerLimitW ? `${s.powerW.toFixed(0)} / ${s.powerLimitW.toFixed(0)} W` : `${s.powerW.toFixed(0)} W`} />
          {/if}
          {#if s.coreMhz != null && show("coreClock")}<Kv label={tr("coreClock")} value={mhz(s.coreMhz)} />{/if}
          {#if s.memMhz != null && show("memClock")}<Kv label={tr("memClock")} value={mhz(s.memMhz)} />{/if}
          {#if show("fan")}
            {#if s.fanPct != null}
              <Kv label={tr("fan")} value="{s.fanPct}%" />
            {:else if s.fanRpm != null}
              <Kv label={tr("fan")} value="{s.fanRpm} rpm" />
            {/if}
          {/if}
        </div>
      </div>
      <ChartStack tile="gpu" defs={chartDefs(gpu)} height={64} who="gpu" />
      {#if (gpu.vramTotal && show("vram")) || (gpu.sharedTotal && show("shared"))}
        <div class="divider"></div>
        <div class="kv">
          {#if gpu.vramTotal && show("vram")}
            <Kv label={tr("vram")} value="{bytes(gpu.vramUsed)} / {bytes(gpu.vramTotal, 0)}" />
          {/if}
          {#if gpu.sharedTotal && show("shared")}
            <Kv label={tr("sharedMem")} value="{bytes(gpu.sharedUsed)} / {bytes(gpu.sharedTotal, 0)}" />
          {/if}
        </div>
      {/if}
    {:else}
      <div class="center"><Donut value={gpu.usage} color="var(--gpu)" size={72} /></div>
      <Spark data={store.series[`gpu.${gpu.luid}.usage`] ?? []} times={store.times} color="var(--gpu)" max={100} fmt={percent} height={22} who="gpu" />
      <p class="sub c" title={gpu.name}>{shortName(gpu.name)}</p>
      <div class="memrow">
        <div class="mh">
          <span class="sub">{mm.label}</span>
          <span class="sub nowrap">{pairBytes(mm.used, mm.total, true)}</span>
        </div>
        <Bar value={pct(mm.used, mm.total)} color="var(--gpu)" height={8} />
      </div>
    {/if}
  </Tile>
{/if}

<style>
  .c {
    text-align: center;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .seg {
    display: flex;
    flex-wrap: wrap;
    padding: 3px;
    border-radius: 11px;
    box-shadow: var(--inset-sm);
    gap: 2px;
    align-self: flex-start;
    max-width: 100%;
  }
  .seg button {
    padding: 3px 9px;
    border-radius: 8px;
    font-size: 11px;
    color: var(--muted);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .seg button.on {
    box-shadow: var(--raised-sm);
    color: var(--text);
    font-weight: 600;
  }
  .memrow {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .mh {
    display: flex;
    justify-content: space-between;
    gap: 6px;
  }
  .nowrap {
    white-space: nowrap;
  }
</style>
