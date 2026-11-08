<script lang="ts">
  import Tile from "../Tile.svelte";
  import Donut from "../Donut.svelte";
  import Spark from "../Spark.svelte";
  import Kv from "../Kv.svelte";
  import ChartStack, { type ChartDef } from "../ChartStack.svelte";
  import { tr } from "../tr";
  import { ghz, cacheSize, percent, celsius, wattsNum } from "../format";
  import { store, type Metrics } from "../metrics.svelte";
  import { settings } from "../settings.svelte";
  import { tooltip, tip } from "../tipstate.svelte";
  import { caps } from "../caps.svelte";

  let { m, adv, span }: { m: Metrics; adv: boolean; span: number } = $props();
  const hw = $derived(store.hw);
  const name = $derived(hw?.cpuName || m.cpuName);
  const temp = $derived(m.cpuTempC);
  const show = (row: string) => settings.shown("cpu", row);
  const badgeKind = $derived(caps.badge("cpu"));
  const clockText = $derived(m.cpuFreqMhz ? ghz(m.cpuFreqMhz) : "");
  const cpuBadge = $derived(
    badgeKind === "none"
      ? ""
      : badgeKind === "usage"
        ? percent(m.cpuUsage)
        : badgeKind === "clock" || temp == null
          ? clockText
          : celsius(temp),
  );
  const tempBadge = $derived(badgeKind === "temp" && temp != null);
  /** Where the temperature / power come from (the tooltip says how far to trust them). */
  const toolTip = (src?: string) => (src === "afterburner" ? tr("srcAfterburner") : src === "hwinfo" ? tr("srcHwinfo") : undefined);
  const tempTip = $derived(m.cpuTempSrc === "acpi" ? tr("tempAcpi") : toolTip(m.cpuTempSrc));
  const powerTip = $derived(toolTip(m.cpuPowerSrc));
  const cpuTone = $derived(!tempBadge || temp == null ? "neutral" : temp < 80 ? "ok" : temp < 92 ? "warn" : "bad");
  /** What the advanced charts can draw (Settings > Modules > Customize, or the chart's own menu). */
  const CHARTS = $derived<Record<string, ChartDef>>({
    usage: { series: "cpu.usage", fmt: percent, max: 100, label: tr("usage"), color: "var(--cpu)" },
    temp: { series: "cpu.temp", fmt: celsius, label: tr("temperature"), color: "var(--cpu)" },
    power: { series: "cpu.power", fmt: wattsNum, label: tr("cpuPower"), color: "var(--cpu)" },
    clock: { series: "cpu.clock", fmt: ghz, label: tr("clock"), color: "var(--cpu)" },
  });
  // More than 16 threads: two rows of bars instead of unreadably thin columns.
  const coreCols = $derived(m.cpuPerCore.length > 16 ? Math.ceil(m.cpuPerCore.length / 2) : m.cpuPerCore.length);
  // Per-core readout, like the charts: instant, and it follows the numbers while the pointer stays on a bar.
  // "CPU n" counts logical processors (threads), the way Afterburner and Task Manager's per-core view do.
  let coreIdx = $state<number | null>(null);
  let coreEl: HTMLElement | null = null;
  let ownsTip = false;
  function coreMove(e: PointerEvent) {
    const el = (e.target as HTMLElement).closest<HTMLElement>(".core");
    coreEl = el;
    coreIdx = el ? Number(el.dataset.i) : null;
  }
  $effect(() => {
    const i = coreIdx;
    const v = i == null ? undefined : m.cpuPerCore[i];
    if (i == null || v == null || !coreEl) {
      if (ownsTip) tooltip.hide();
      ownsTip = false;
      return;
    }
    const mhz = m.cpuCoreMhz?.[i];
    tooltip.show(`CPU ${i + 1} · ${percent(v)}${mhz ? ` · ${ghz(mhz)}` : ""}`, coreEl.getBoundingClientRect(), true);
    ownsTip = true;
  });
  $effect(() => () => {
    if (ownsTip) tooltip.hide();
  });

  const cache = $derived(
    hw && (hw.cpuL2Kb || hw.cpuL3Kb)
      ? [hw.cpuL2Kb && `L2 ${cacheSize(hw.cpuL2Kb)}`, hw.cpuL3Kb && `L3 ${cacheSize(hw.cpuL3Kb)}`]
          .filter(Boolean)
          .join(" · ")
      : "",
  );
</script>

<!-- A popup hidden while the pointer was on a bar must not come back with a stuck readout. -->
<svelte:window onblur={() => (coreIdx = null)} />
<svelte:document onvisibilitychange={() => (coreIdx = null)} />

<Tile
  title={tr("cpu")}
  icon="cpu"
  color="var(--cpu)"
  {span}
  badge={adv && cpuBadge ? cpuBadge : undefined}
  badgeTone={cpuTone}
  badgeTip={tempBadge ? (tempTip ?? "") : ""}
  badgePick={caps.quick("cpu", "badge")}
>
  {#if adv}
    {#if show("model")}<p class="model" title={name}>{name}</p>{/if}
    <div class="split">
      <div class="vis" use:tip={tr("cpuUsageTip")}>
        <Donut value={m.cpuUsage} color="var(--cpu)" size={96} />
      </div>
      <div class="kv">
        {#if show("cores")}<Kv label={tr("coresLabel")} value="{m.cpuPhysical} / {m.cpuCores}" />{/if}
        {#if show("clock")}
          <Kv
            label={tr("clock")}
            value={m.cpuFreqMhz
              ? `${ghz(m.cpuFreqMhz)}${hw?.cpuMaxMhz ? ` · ${tr("base")} ${ghz(hw.cpuMaxMhz)}` : ""}`
              : "—"}
          />
        {/if}
        {#if cache && show("cache")}<Kv label={tr("cache")} value={cache} />{/if}
        {#if m.cpuTempC != null && show("temp")}
          <Kv label={tr("temperature")} value={celsius(m.cpuTempC)} title={tempTip} />
        {/if}
        {#if m.cpuPowerW != null && show("power")}
          <Kv label={tr("cpuPower")} value="{m.cpuPowerW.toFixed(1)} W" title={powerTip} />
        {/if}
      </div>
    </div>
    <ChartStack tile="cpu" defs={CHARTS} height={64} who="cpu" />
    {#if show("perCore")}
      <div class="divider"></div>
      <span class="sub">{tr("perCore")}</span>
      <div
        class="cores"
        style="grid-template-columns:repeat({coreCols},1fr)"
        role="presentation"
        onpointermove={coreMove}
        onpointerleave={() => (coreIdx = null)}
      >
        {#each m.cpuPerCore as c, i (i)}
          <div class="core" class:hot={coreIdx === i} data-i={i}>
            <i style="height:{Math.max(4, c)}%"></i>
          </div>
        {/each}
      </div>
    {/if}
  {:else}
    <div class="center" use:tip={tr("cpuUsageTip")}><Donut value={m.cpuUsage} color="var(--cpu)" size={72} /></div>
    <Spark data={store.series["cpu.usage"] ?? []} times={store.times} color="var(--cpu)" max={100} fmt={percent} height={22} who="cpu" />
    <p class="sub c">
      {m.cpuCores} {tr("cores")}{m.cpuFreqMhz ? ` · ${ghz(m.cpuFreqMhz)}` : ""}
    </p>
  {/if}
</Tile>

<style>
  .c {
    text-align: center;
  }
  .cores {
    display: grid;
    gap: 4px;
    grid-auto-rows: 30px;
  }
  .core {
    display: flex;
    align-items: flex-end;
    border-radius: 6px;
    background: var(--bg);
    box-shadow: var(--inset-sm);
    padding: 2px;
    overflow: hidden;
  }
  .core.hot {
    box-shadow: var(--inset-sm), 0 0 0 1.5px color-mix(in srgb, var(--cpu) 70%, transparent);
  }
  .core i {
    display: block;
    width: 100%;
    border-radius: 3px;
    background: var(--cpu);
  }
</style>
