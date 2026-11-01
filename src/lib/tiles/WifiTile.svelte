<script lang="ts">
  import Tile from "../Tile.svelte";
  import Donut from "../Donut.svelte";
  import Kv from "../Kv.svelte";
  import { lower, tr } from "../tr";
  import type { Metrics } from "../metrics.svelte";
  import { settings } from "../settings.svelte";

  let { m, adv, span }: { m: Metrics; adv: boolean; span: number } = $props();
  const w = $derived(m.wifi);
  const show = (row: string) => settings.shown("wifi", row);
  /** Not connected: "Off" (radio switched off) or "Not connected", with what the adapter can see. */
  const state = $derived(w && !w.connected ? tr(w.radioOff ? "wifiOff" : "wifiNotConnected") : "");
  const nearby = $derived(
    w && !w.connected && !w.radioOff && w.networks
      ? tr("wifiNearbyN").replace("{n}", String(w.networks))
      : "",
  );
</script>

{#if w}
  <Tile
    title={tr("wifi")}
    icon="wifi"
    color="var(--cpu)"
    {span}
    badge={adv && settings.badgeOf("wifi") !== "none" ? (w.connected ? `${w.signal}%` : lower(state)) : undefined}
    badgeTone={!w.connected ? "neutral" : w.signal >= 60 ? "ok" : w.signal >= 35 ? "warn" : "bad"}
  >
    {#if w.connected}
      {#if adv}
        <div class="split">
          <div class="vis"><Donut value={w.signal} color="var(--cpu)" size={88} /></div>
          <div class="kv">
            {#if show("ssid")}<Kv label={tr("ssid")} value={w.ssid || "—"} />{/if}
            {#if show("signal")}<Kv label={tr("signal")} value="{w.signal}%" />{/if}
            {#if w.standard && show("standard")}<Kv label={tr("standard")} value={w.standard} />{/if}
            {#if show("rate")}<Kv label={tr("rate")} value="↓ {w.rxMbps} / ↑ {w.txMbps} Mbps" />{/if}
          </div>
        </div>
      {:else}
        <div class="center"><Donut value={w.signal} color="var(--cpu)" size={72} /></div>
        <p class="sub c" title={w.ssid}>{w.ssid || tr("wifi")}</p>
        <p class="sub c">{w.rxMbps} / {w.txMbps} Mbps</p>
      {/if}
    {:else if adv}
      <div class="kv">
        <Kv label={tr("wifiState")} value={state} />
        {#if w.adapter}<Kv label={tr("adapter")} value={w.adapter} wrap />{/if}
        {#if !w.radioOff}
          <Kv label={tr("wifiNearby")} value={String(w.networks)} />
          {#if w.bestSsid}<Kv label={tr("wifiStrongest")} value="{w.bestSsid} · {w.bestSignal}%" wrap />{/if}
        {/if}
      </div>
    {:else}
      <p class="state">{state}</p>
      {#if nearby}<p class="sub c">{nearby}</p>{/if}
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
  .state {
    text-align: center;
    font-size: 13px;
    font-weight: 600;
    color: var(--muted);
    padding: 10px 0 2px;
  }
</style>
