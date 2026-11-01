<script lang="ts">
  import Tile from "../Tile.svelte";
  import Bar from "../Bar.svelte";
  import Kv from "../Kv.svelte";
  import Spark from "../Spark.svelte";
  import { tr, dur } from "../tr";
  import { watts, wh, pct, percent, wattsNum } from "../format";
  import { store, type Metrics } from "../metrics.svelte";
  import { settings } from "../settings.svelte";
  import Icon from "../Icon.svelte";
  import { tip } from "../tipstate.svelte";

  let { m, adv, span, onHistory }: { m: Metrics; adv: boolean; span: number; onHistory?: () => void } = $props();
  const b = $derived(m.battery);
  const d = $derived(b.detail);

  /** What the battery is doing right now. */
  const mode = $derived<"charging" | "full" | "plugged" | "discharging">(
    b.charging ? "charging" : b.plugged ? (b.percent >= 99 ? "full" : "plugged") : "discharging",
  );
  const low = $derived(mode === "discharging" && b.percent <= 20);
  const color = $derived(low ? "var(--danger)" : "var(--bat)");

  // Positive rate = charging, negative = discharging (milliwatts).
  const rate = $derived(d?.rateMw ?? 0);
  const power = $derived(rate ? `${rate > 0 ? "+" : "−"}${watts(rate)}` : "");
  const powerLabel = $derived(
    mode === "charging" ? tr("chargePower") : mode === "discharging" ? tr("powerDraw") : tr("power"),
  );

  /** Estimates, rounded to whole minutes so the text doesn't flicker. */
  const round = (secs: number) => Math.max(60, Math.round(secs / 60) * 60);
  const secsToFull = $derived(
    d && mode === "charging" && rate > 0 && d.fullMwh > d.currentMwh && d.currentMwh > 0
      ? round(((d.fullMwh - d.currentMwh) / rate) * 3600)
      : null,
  );
  const secsLeft = $derived(
    mode !== "discharging"
      ? null
      : b.remainingSecs != null
        ? round(b.remainingSecs)
        : d && rate < 0 && d.currentMwh > 0
          ? round((d.currentMwh / -rate) * 3600)
          : null,
  );

  /** The state in words; the (approximate) time estimate goes on its own line below it. */
  const status = $derived(
    mode === "charging"
      ? tr("charging")
      : mode === "full"
        ? tr("fullCharged")
        : mode === "plugged"
          ? tr("onAc")
          : tr("onBattery"),
  );
  const estimate = $derived(
    secsToFull
      ? `${tr("untilFull")} ~${dur(secsToFull)}`
      : secsLeft
        ? `~${dur(secsLeft)} ${tr("remaining")}`
        : "",
  );

  const health = $derived(d && d.designMwh ? Math.min(100, pct(d.fullMwh, d.designMwh)) : null);
  const capacity = $derived(d && d.fullMwh ? `${wh(d.currentMwh || 0)} / ${wh(d.fullMwh)}` : "");
  /** Power charts only make sense once energy has actually flowed. */
  const powerSeries = $derived(store.series["bat.power"] ?? []);
  const hasFlow = $derived(powerSeries.some((v) => v != null && v > 0.05));
  const show = (row: string) => settings.shown("battery", row);

  const badge = $derived(
    mode === "charging"
      ? { icon: "bolt", text: power || tr("charging"), tone: "ok" as const }
      : mode === "discharging"
        ? { icon: "arrowDown", text: power || tr("onBattery"), tone: low ? ("bad" as const) : ("warn" as const) }
        : { icon: "plug", text: mode === "full" ? tr("fullCharged") : tr("onAc"), tone: "ok" as const },
  );
</script>

<Tile
  title={tr("battery")}
  icon="battery"
  color="var(--bat)"
  {span}
  badge={adv && settings.badgeOf("battery") !== "none" ? badge.text : undefined}
  badgeIcon={badge.icon}
  badgeTone={badge.tone}
>
  {#snippet actions()}
    {#if onHistory}
      <button class="icon-btn sm" aria-label={tr("batHistory")} use:tip={tr("batHistory")} onclick={onHistory}>
        <Icon name="history" size={14} />
      </button>
    {/if}
  {/snippet}
  {#if adv}
    <div class="split">
      <div class="vis">
        <b class="big">{b.percent}%</b>
        <Bar value={b.percent} {color} />
        <span class="sub">{status}</span>
        {#if estimate}<span class="sub">{estimate}</span>{/if}
      </div>
      <div class="kv">
        {#if health !== null && d && show("health")}
          <Kv label={tr("health")} value="{health.toFixed(0)}%" title="{wh(d.fullMwh)} / {wh(d.designMwh)}" />
        {/if}
        {#if capacity && show("capacity")}<Kv label={tr("energyLeft")} value={capacity} />{/if}
        {#if power && show("power")}<Kv label={powerLabel} value={power} />{/if}
        {#if secsToFull && show("time")}<Kv label={tr("untilFull")} value="~{dur(secsToFull)}" />{/if}
        {#if secsLeft && show("time")}<Kv label={tr("timeLeft")} value="~{dur(secsLeft)}" />{/if}
        {#if d?.voltageMv && show("voltage")}<Kv label={tr("voltage")} value="{(d.voltageMv / 1000).toFixed(2)} V" />{/if}
        {#if d?.cycles && show("cycles")}<Kv label={tr("cycles")} value={String(d.cycles)} />{/if}
      </div>
    </div>
    {#if (hasFlow && show("powerChart")) || show("chargeChart")}<div class="divider"></div>{/if}
    {#if hasFlow && show("powerChart")}
      <div class="chart">
        <div class="ch"><span class="sub">{powerLabel}</span><b>{power || "—"}</b></div>
        <Spark data={powerSeries} times={store.times} windowSecs={settings.historySecs} color="var(--disk)" fmt={wattsNum} height={48} detail />
      </div>
    {/if}
    {#if show("chargeChart")}
      <div class="chart">
        <div class="ch"><span class="sub">{tr("charge")}</span><b>{b.percent}%</b></div>
        <Spark data={store.chargeHist} times={store.chargeTimes} windowSecs={600} gapMs={12000} color="var(--bat)" max={100} fmt={percent} height={48} detail />
      </div>
    {/if}
  {:else}
    <div class="stack c">
      <b class="big">{b.percent}%</b>
      <Bar value={b.percent} {color} />
      <span class="sub">{status}</span>
      {#if estimate}<span class="sub">{estimate}</span>{/if}
      {#if power}
        <div class="ch"><span class="sub">{powerLabel}</span><b>{power}</b></div>
        {#if hasFlow}<Spark data={powerSeries} times={store.times} color="var(--disk)" fmt={wattsNum} height={22} />{/if}
      {/if}
      {#if capacity}<span class="sub n">{capacity}</span>{/if}
    </div>
  {/if}
</Tile>

<style>
  .big {
    font-size: 22px;
    font-weight: 650;
  }
  .c {
    align-items: stretch;
    text-align: center;
    gap: 6px;
  }
  .vis {
    align-items: stretch;
    text-align: center;
  }
  .chart {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .ch {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 6px;
    font-size: 12px;
  }
  .ch b {
    font-weight: 650;
  }
  .n {
    font-size: 10px;
  }
</style>
