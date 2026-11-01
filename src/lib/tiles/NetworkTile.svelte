<script lang="ts">
  import Tile from "../Tile.svelte";
  import Spark from "../Spark.svelte";
  import Icon from "../Icon.svelte";
  import Kv from "../Kv.svelte";
  import { tr } from "../tr";
  import { bytes, rate } from "../format";
  import { store, type Metrics } from "../metrics.svelte";
  import { settings } from "../settings.svelte";

  let { m, adv, span }: { m: Metrics; adv: boolean; span: number } = $props();

  const info = $derived.by(() => {
    const n = m.netAdapter;
    if (!n) return null;
    const hw = store.hw?.adapters.find((a) => a.description === n.name || a.name === n.name);
    const speed = hw?.linkMbps
      ? hw.linkMbps >= 1000
        ? `${hw.linkMbps / 1000} Gbps`
        : `${hw.linkMbps} Mbps`
      : "";
    const kind = hw?.kind === "wifi" ? "Wi-Fi" : hw?.kind === "ethernet" ? "Ethernet" : "";
    return { label: hw?.name ?? n.name, ip: n.ip, speed, badge: [kind, speed].filter(Boolean).join(" · ") };
  });
  const totals = $derived([
    { label: tr("today"), down: m.netTodayDown, up: m.netTodayUp },
    { label: tr("session"), down: m.netSessionDown, up: m.netSessionUp },
    { label: tr("sinceBoot"), down: m.netBootDown, up: m.netBootUp },
  ]);
  const show = (row: string) => settings.shown("network", row);
  const badge = $derived(settings.badgeOf("network") === "link" && info?.badge ? info.badge : undefined);
  const kvShown = $derived(info != null && (show("adapter") || show("ip") || (show("linkSpeed") && !!info.speed)));
</script>

<Tile title={tr("network")} icon="network" color="var(--net)" {span} badge={adv ? badge : undefined}>
  {#if adv}
    <div class="stack">
      <div class="rate">
        <span class="lab"><Icon name="arrowDown" size={12} />{tr("download")}</span>
        <b>{rate(m.netDown)}</b>
      </div>
      {#if show("downChart")}
        <Spark data={store.series["net.down"] ?? []} times={store.times} windowSecs={settings.historySecs} color="var(--net)" fmt={rate} bytes floor={1024} height={56} detail />
      {/if}
      <div class="rate">
        <span class="lab"><Icon name="arrowUp" size={12} />{tr("upload")}</span>
        <b>{rate(m.netUp)}</b>
      </div>
      {#if show("upChart")}
        <Spark data={store.series["net.up"] ?? []} times={store.times} windowSecs={settings.historySecs} color="var(--disk)" fmt={rate} bytes floor={1024} height={56} detail />
      {/if}
    </div>
    {#if kvShown && info}
      <div class="divider"></div>
      <div class="kv">
        {#if show("adapter")}<Kv label={tr("adapter")} value={info.label} />{/if}
        {#if show("ip")}<Kv label={tr("ip")} value={info.ip} />{/if}
        {#if info.speed && show("linkSpeed")}<Kv label={tr("linkSpeed")} value={info.speed} />{/if}
      </div>
    {/if}
    {#if show("totals")}
      <div class="divider"></div>
      <div class="totals">
        <span></span>
        <span class="sub h"><Icon name="arrowDown" size={11} />{tr("download")}</span>
        <span class="sub h"><Icon name="arrowUp" size={11} />{tr("upload")}</span>
        {#each totals as t (t.label)}
          <span class="sub">{t.label}</span>
          <b>{bytes(t.down)}</b>
          <b>{bytes(t.up)}</b>
        {/each}
      </div>
    {/if}
  {:else}
    <div class="stack">
      <div class="rate">
        <span class="lab"><Icon name="arrowDown" size={12} />{tr("download")}</span>
        <b>{rate(m.netDown)}</b>
      </div>
      <Spark data={store.series["net.down"] ?? []} times={store.times} color="var(--net)" fmt={rate} bytes floor={1024} />
      <div class="rate">
        <span class="lab"><Icon name="arrowUp" size={12} />{tr("upload")}</span>
        <b>{rate(m.netUp)}</b>
      </div>
      <Spark data={store.series["net.up"] ?? []} times={store.times} color="var(--disk)" fmt={rate} bytes floor={1024} />
      <div class="today">
        <span class="sub">{tr("today")}</span>
        <span class="tot">↓ {bytes(m.netTodayDown)} · ↑ {bytes(m.netTodayUp)}</span>
      </div>
    </div>
  {/if}
</Tile>

<style>
  .today {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1px;
    margin-top: -2px;
  }
  .tot {
    font-weight: 600;
    font-size: 10.5px;
    white-space: nowrap;
  }
  .totals {
    display: grid;
    grid-template-columns: auto 1fr 1fr;
    gap: 5px 12px;
    font-size: 11.5px;
    align-items: center;
  }
  .totals b {
    font-weight: 650;
  }
  .h {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
</style>
