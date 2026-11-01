<script lang="ts">
  import Tile from "../Tile.svelte";
  import Bar from "../Bar.svelte";
  import Kv from "../Kv.svelte";
  import { lower, tr } from "../tr";
  import type { Key } from "../i18n";
  import Spark from "../Spark.svelte";
  import { rate, pct, percent, size, sizePair, namesSize, celsius, trPossessive } from "../format";
  import { settings } from "../settings.svelte";
  import { caps } from "../caps.svelte";
  import { store, type Metrics, type DiskHealth } from "../metrics.svelte";

  let { m, adv, span }: { m: Metrics; adv: boolean; span: number } = $props();
  const drives = $derived(adv ? m.disks : m.disks.slice(0, 3));
  const label = (mount: string) => mount.replace(/[\\/]+$/, "") || mount;
  const show = (row: string) => !adv || settings.shown("disk", row);
  const chartKind = $derived(caps.chart("disk"));
  const CHARTS: Record<string, { fmt: (v: number) => string; max?: number; bytes?: boolean }> = {
    active: { fmt: percent, max: 100 },
    read: { fmt: rate, bytes: true },
    write: { fmt: rate, bytes: true },
  };
  // grafik adı, ayarlardaki seçenekle aynı sözcük (Etkinlik / Okuma / Yazma)
  const chart = $derived(CHARTS[chartKind]);
  /** A nearly full drive turns its bar red (Windows itself warns at 90 %). */
  const FULL_PCT = 90;
  const kvShown = $derived(
    show("active") || show("read") || show("write") || (show("models") && (store.hw?.drives.length ?? 0) > 0),
  );

  // --- Disk sağlığı (NVMe sağlık kaydı + sıcaklık; diskhealth.rs) ---
  const health = $derived(m.diskHealth ?? []);
  const fill = (key: Key, n: string | number, m2: string | number = "") =>
    tr(key).replace("{n}", String(n)).replace("{s}", trPossessive(n)).replace("{m}", String(m2));
  const count = (n: number) => n.toLocaleString(settings.lang === "tr" ? "tr-TR" : "en-US");
  const modelOf = (h: DiskHealth) =>
    store.hw?.drives.find((d) => d.index === h.index)?.model || `${tr("drives")} ${h.index}`;
  const STATUS: Record<string, { key: Key; tone: string }> = {
    good: { key: "healthGood", tone: "ok" },
    warn: { key: "healthWarn", tone: "warn" },
    bad: { key: "healthBad", tone: "bad" },
  };
  function tempTone(h: DiskHealth): string {
    if (h.tempC == null) return "";
    if (h.critC != null && h.tempC >= h.critC) return "bad";
    if (h.warnC != null && h.tempC >= h.warnC) return "warn";
    return "";
  }
  /** Decimal units, like the drive makers (TBW warranty) and Samsung Magician / CrystalDiskInfo: 53.9 TB, not 49.0. */
  const decimal = (n: number) => (n >= 1e12 ? `${(n / 1e12).toFixed(1)} TB` : `${(n / 1e9).toFixed(0)} GB`);
  function summary(h: DiskHealth): string {
    const n = h.nvme;
    if (!n) return "";
    return [fill("wearUsed", n.usedPct), fill("dataWritten", decimal(n.writtenBytes)), fill("hoursOn", count(n.powerOnHours))].join(" · ");
  }
  function details(h: DiskHealth): string {
    const n = h.nvme;
    const lines = [modelOf(h)];
    if (n) {
      if (n.criticalWarning) lines.push(tr("criticalWarning"));
      lines.push(
        fill("spareLeft", n.sparePct, n.spareThresholdPct),
        fill("mediaErrors", n.mediaErrors),
        fill("unsafeShutdowns", n.unsafeShutdowns),
        fill("powerCycles", n.powerCycles.toLocaleString()),
        fill("dataRead", decimal(n.readBytes)),
      );
    }
    if (h.warnC != null && h.critC != null) lines.push(fill("tempLimits", celsius(h.warnC), celsius(h.critC)));
    return lines.join("\n");
  }
  const hottest = $derived(
    health.filter((h) => h.tempC != null).sort((a, b) => (b.tempC as number) - (a.tempC as number))[0],
  );
  const badgeKind = $derived(caps.badge("disk"));
</script>

<Tile
  title={tr("disk")}
  icon="disk"
  color="var(--disk)"
  {span}
  badge={!adv
    ? undefined
    : badgeKind === "active"
      ? `${m.diskActive.toFixed(0)}% ${lower(tr("active"))}`
      : badgeKind === "temp" && hottest
        ? celsius(hottest.tempC as number)
        : undefined}
  badgeTone={badgeKind === "temp" && hottest
    ? ((tempTone(hottest) || "ok") as "ok" | "warn" | "bad")
    : m.diskActive > 90
      ? "bad"
      : m.diskActive > 70
        ? "warn"
        : "neutral"}
  badgeTip={badgeKind === "temp" && hottest ? `${tr("driveTemp")} · ${modelOf(hottest)}` : undefined}
>
  {#each show("drives") ? drives : [] as d, i (i)}
    <div class="bar-row">
      <div class="bar-head">
        <span>
          {label(d.mount)}
          {#if adv}<em>{[d.kind, d.fs].filter(Boolean).join(" · ")}</em>{/if}
        </span>
        <span class="sub">
          {adv ? `${size(d.total - d.used)} ${tr("free")} · ` : ""}{sizePair(d.used, d.total)}
        </span>
      </div>
      <Bar value={pct(d.used, d.total)} color={pct(d.used, d.total) >= FULL_PCT ? "var(--danger)" : "var(--disk)"} />
    </div>
  {/each}

  {#if adv}
    {#if chart}
      <Spark
        data={store.series[`disk.${chartKind}`] ?? []}
        times={store.times}
        windowSecs={settings.historySecs}
        color="var(--disk)"
        max={chart.max}
        fmt={chart.fmt}
        bytes={chart.bytes}
        floor={chart.bytes ? 1024 : 1}
        label={tr(chartKind as Key)}
        height={56}
        who="disk"
        detail
      />
    {/if}
    {#if kvShown}
      <div class="divider"></div>
      <div class="kv">
        {#if show("active")}<Kv label={tr("active")} value={percent(m.diskActive)} />{/if}
        {#if show("read")}<Kv label={tr("read")} value={rate(m.diskRead)} />{/if}
        {#if show("write")}<Kv label={tr("write")} value={rate(m.diskWrite)} />{/if}
        {#if show("models")}
          {#each store.hw?.drives ?? [] as p, i (i)}
            <Kv
              label={p.media || tr("drives")}
              value={[p.model, namesSize(p.model) ? "" : size(p.size), p.bus].filter(Boolean).join(" · ")}
              wrap
            />
          {/each}
        {/if}
      </div>
    {/if}
    {#if show("health") && health.length}
      <div class="divider"></div>
      <span class="sub">{tr("diskHealth")}</span>
      {#each health as h (h.index)}
        <div class="health" title={details(h)}>
          <div class="hrow">
            <span class="hname">{modelOf(h)}</span>
            <span class="hval">
              {#if STATUS[h.status]}<b class="pill {STATUS[h.status].tone}">{tr(STATUS[h.status].key)}</b>{/if}
              {#if h.tempC != null}<span class="temp {tempTone(h)}">{celsius(h.tempC)}</span>{/if}
            </span>
          </div>
          {#if summary(h)}<span class="hsum">{summary(h)}</span>{/if}
        </div>
      {/each}
    {/if}
  {:else}
    <div class="rw">
      <span>{tr("read")} <b>{rate(m.diskRead)}</b></span>
      <span>{tr("write")} <b>{rate(m.diskWrite)}</b></span>
    </div>
  {/if}
</Tile>

<style>
  .bar-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .bar-head {
    display: flex;
    justify-content: space-between;
    gap: 6px;
    font-weight: 600;
    font-size: 12px;
  }
  em {
    font-style: normal;
    font-weight: 400;
    font-size: 10.5px;
    color: var(--muted);
    margin-left: 4px;
  }
  .rw {
    display: flex;
    justify-content: space-between;
    font-size: 10.5px;
    color: var(--muted);
    margin-top: auto;
  }
  .rw b {
    display: block;
    color: var(--text);
    font-weight: 600;
  }
  .health {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .hrow {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
    min-width: 0;
  }
  .hname {
    font-weight: 600;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hval {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .pill {
    font-size: 10px;
    font-weight: 700;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--bg);
    box-shadow: var(--inset-sm);
  }
  .pill.ok {
    color: var(--ok-text);
  }
  .pill.warn {
    color: var(--warn-text);
  }
  .pill.bad {
    color: var(--danger);
  }
  .temp {
    font-weight: 600;
  }
  .temp.warn {
    color: var(--warn-text);
  }
  .temp.bad {
    color: var(--danger);
  }
  .hsum {
    font-size: 10.5px;
    color: var(--muted);
    min-width: 0;
    overflow-wrap: anywhere;
  }
</style>
