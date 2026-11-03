<script lang="ts">
  import Tile from "../Tile.svelte";
  import Donut from "../Donut.svelte";
  import Spark from "../Spark.svelte";
  import Kv from "../Kv.svelte";
  import { tr } from "../tr";
  import { bytes, pct, pairBytes, percent } from "../format";
  import { store, type Metrics } from "../metrics.svelte";
  import { settings } from "../settings.svelte";
  import { caps } from "../caps.svelte";

  let { m, adv, span }: { m: Metrics; adv: boolean; span: number } = $props();
  const hw = $derived(store.hw);
  const mods = $derived(hw?.memModules ?? []);
  const type = $derived.by(() => {
    const f = mods[0];
    if (!f) return "";
    const speed = f.configuredMts || f.ratedMts;
    return [f.kind, speed ? `${speed} MT/s` : ""].filter(Boolean).join(" ");
  });
  const layout = $derived.by(() => {
    if (!mods.length) return "";
    const groups = new Map<number, number>();
    for (const x of mods) groups.set(x.capacity, (groups.get(x.capacity) ?? 0) + 1);
    const parts = [...groups].map(([cap, n]) => `${n} × ${bytes(cap, 0)}`).join(" + ");
    return hw?.memSlots ? `${parts} (${mods.length}/${hw.memSlots} ${tr("slots")})` : parts;
  });
  const maker = $derived(mods.find((x) => x.maker && x.maker !== "Unknown")?.maker ?? "");
  const ramPct = $derived(pct(m.memUsed, m.memTotal));
  const swapPct = $derived(pct(m.swapUsed, m.swapTotal));
  const size = $derived(adv ? 88 : 66);
  /** RAM and swap together: what the system could ever hand out, and how much of it is in use. */
  const combined = $derived(pairBytes(m.memUsed + m.swapUsed, m.memTotal + m.swapTotal));
  /** The badge colour warns when little RAM is left (Windows keeps "used" high on purpose) or the commit nears its limit. */
  const free = $derived(pct(m.memAvailable, m.memTotal));
  const commitPct = $derived(pct(m.commitUsed, m.commitLimit));
  const tone = $derived(
    free < 10 || commitPct >= 97
      ? ("bad" as const)
      : free < 25 || commitPct >= 90
        ? ("warn" as const)
        : ("neutral" as const),
  );

  const show = (row: string) => settings.shown("memory", row);
  const badgeKind = $derived(caps.badge("memory"));
  const badge = $derived(
    badgeKind === "none"
      ? undefined
      : badgeKind === "ram"
        ? pairBytes(m.memUsed, m.memTotal)
        : badgeKind === "swap" && m.swapTotal
          ? pairBytes(m.swapUsed, m.swapTotal)
          : badgeKind === "commit" && m.commitLimit
            ? pairBytes(m.commitUsed, m.commitLimit)
            : combined,
  );
  /** Which number the badge really shows (swap and commit fall back to RAM + swap when the machine has none). */
  const shown = $derived(
    badgeKind === "ram" || (badgeKind === "swap" && m.swapTotal) || (badgeKind === "commit" && m.commitLimit)
      ? badgeKind
      : "combined",
  );
  const badgeLabel = $derived(
    shown === "commit" ? tr("commit") : shown === "ram" ? tr("ram") : shown === "swap" ? tr("swap") : tr("memCombined"),
  );
  const badgeTip = $derived(shown === "commit" ? tr("commitTip") : shown === "combined" ? tr("memCombinedTip") : tr("memToneTip"));
  const chartKind = $derived(caps.chart("memory"));
  /** Swap only when the machine has any; otherwise the chart falls back to RAM. */
  const chartKey = $derived(chartKind === "none" ? null : chartKind === "swap" && m.swapTotal ? "mem.swap" : "mem.ram");
  const showSwap = $derived(m.swapTotal > 0 && (!adv || show("swap")));
</script>

<Tile
  title={tr("memory")}
  icon="memory"
  color="var(--mem)"
  {span}
  badge={adv ? badge : undefined}
  {badgeLabel}
  badgeTone={tone}
  {badgeTip}
  badgePick={caps.quick("memory", "badge")}
>
  <div class="pair">
    <div class="cell">
      <Donut value={ramPct} color="var(--mem)" {size} />
      <span class="cap">{tr("ram")}</span>
      <span class="sub n">{pairBytes(m.memUsed, m.memTotal, !adv)}</span>
    </div>
    {#if showSwap}
      <div class="cell">
        <Donut value={swapPct} color="var(--net)" {size} />
        <span class="cap">{tr("swap")}</span>
        <span class="sub n">{pairBytes(m.swapUsed, m.swapTotal, !adv)}</span>
      </div>
    {/if}
  </div>
  {#if !adv}
    <Spark data={store.series["mem.ram"] ?? []} times={store.times} color="var(--mem)" max={100} fmt={percent} height={24} who="mem" />
  {:else}
    {#if chartKey}
      <Spark
        data={store.series[chartKey] ?? []}
        times={store.times}
        windowSecs={settings.historySecs}
        color={chartKey === "mem.swap" ? "var(--net)" : "var(--mem)"}
        max={100}
        fmt={percent}
        label={tr(chartKey === "mem.swap" ? "swap" : "ram")}
        pick={caps.quick("memory", "chart")}
        height={56}
        who="mem"
        detail
      />
    {/if}
    {@const rows = [
      show("used") && [tr("used"), bytes(m.memUsed)],
      show("available") && [tr("available"), bytes(m.memAvailable)],
      show("cached") && m.memCached && [tr("cachedMem"), bytes(m.memCached)],
      show("commit") && m.commitLimit && [tr("commit"), `${bytes(m.commitUsed)} / ${bytes(m.commitLimit)}`, tr("commitTip")],
      show("type") && type && [tr("memType"), type],
      show("modules") && layout && [tr("memModules"), layout],
      show("maker") && maker && [tr("maker"), maker],
    ].filter((r): r is string[] => Array.isArray(r))}
    {#if rows.length}
      <div class="divider"></div>
      <div class="kv">
        {#each rows as [label, value, title] (label)}<Kv {label} {value} {title} />{/each}
      </div>
    {/if}
  {/if}
</Tile>

<style>
  .pair {
    display: flex;
    justify-content: space-around;
    gap: 4px;
  }
  .cell {
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    min-width: 0;
  }
  .cap {
    font-size: 11px;
    font-weight: 600;
    margin-top: 3px;
    white-space: nowrap;
  }
  .n {
    font-size: 9.5px;
    white-space: nowrap;
  }
</style>
