<script lang="ts">
  import Tile from "../Tile.svelte";
  import Bar from "../Bar.svelte";
  import Kv from "../Kv.svelte";
  import { lower, tr } from "../tr";
  import type { Key } from "../i18n";
  import ChartStack, { type ChartDef } from "../ChartStack.svelte";
  import Icon from "../Icon.svelte";
  import OptionMenu from "../OptionMenu.svelte";
  import { rate, pct, percent, size, sizePair, namesSize, celsius, trPossessive } from "../format";
  import { MAX_TABS, ownsMount, tabLabel, fullLabel, seriesPrefix } from "../disks";
  import type { QuickPick } from "../tiledetail";
  import { settings } from "../settings.svelte";
  import { caps } from "../caps.svelte";
  import { store, type Metrics, type DiskHealth } from "../metrics.svelte";

  let { m, adv, span }: { m: Metrics; adv: boolean; span: number } = $props();
  const label = (mount: string) => mount.replace(/[\\/]+$/, "") || mount;
  const show = (row: string) => !adv || settings.shown("disk", row);

  // --- Several physical disks: tabs (or a list when there are many) to look at one of them, and a line per disk
  const phys = $derived(m.physDisks ?? []);
  const modelOfDisk = (index: number) => store.hw?.drives.find((d) => d.index === index)?.model;
  const diskTab = (p: (typeof phys)[number]) => tabLabel(p, modelOfDisk(p.index), tr("disk"));
  const diskFull = (p: (typeof phys)[number]) => fullLabel(p, modelOfDisk(p.index), tr("disk"));
  const tabsOn = $derived(adv && phys.length >= 2 && settings.shown("disk", "tabs"));
  /** The disk index being looked at; `null` = all together. A disk that is gone shows everything again. */
  let chosen = $state<number | null>(null);
  const sel = $derived(tabsOn && chosen != null ? phys.find((p) => p.index === chosen) : undefined);
  const diskPick = $derived<QuickPick>({
    options: [{ id: "all", label: tr("allDisks") }, ...phys.map((p) => ({ id: String(p.index), label: diskFull(p) }))],
    value: sel ? String(sel.index) : "all",
    set: (id) => (chosen = id === "all" ? null : Number(id)),
    tip: tr("pickDiskTip"),
  });
  const all = $derived(adv ? m.disks : m.disks.slice(0, 3));
  /** The volumes shown: those of the chosen disk, or all. */
  const drives = $derived(sel ? all.filter((d) => ownsMount(sel, d.mount)) : all);
  /** The numbers shown: the chosen disk's, or the sum of all. */
  const cur = $derived(sel ?? { active: m.diskActive, read: m.diskRead, write: m.diskWrite });
  /** With several disks the numbers of "all" are sums: say so on hover. */
  const sum = $derived(!sel && phys.length >= 2 ? tr("diskTotalTip") : undefined);
  const models = $derived((store.hw?.drives ?? []).filter((p) => !sel || p.index === sel.index));
  // chart names are the same words as the options in Settings (Active / Read / Write)
  const CHARTS = $derived.by<Record<string, ChartDef>>(() => {
    const sp = seriesPrefix(sel);
    const color = "var(--disk)";
    return {
      active: { series: `${sp}active`, fmt: percent, max: 100, label: tr("active"), color },
      read: { series: `${sp}read`, fmt: rate, bytes: true, floor: 1024, label: tr("read"), color },
      write: { series: `${sp}write`, fmt: rate, bytes: true, floor: 1024, label: tr("write"), color },
    };
  });
  /** A nearly full drive turns its bar red (Windows itself warns at 90 %). */
  const FULL_PCT = 90;
  const kvShown = $derived(
    show("active") || show("read") || show("write") || (show("models") && models.length > 0),
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
  const shownHealth = $derived(sel ? health.filter((h) => h.index === sel.index) : health);
  /** The badge follows what the tile shows: all disks, or the chosen one. */
  const hottest = $derived(
    shownHealth.filter((h) => h.tempC != null).sort((a, b) => (b.tempC as number) - (a.tempC as number))[0],
  );
  const badgeKind = $derived(caps.badge("disk"));
</script>

{#snippet pdRow(p: (typeof phys)[number])}
  <span class="pdh">
    <span class="pdn"><b>{p.letters || diskTab(p)}</b>{#if p.letters && modelOfDisk(p.index)}<em>{modelOfDisk(p.index)}</em>{/if}</span>
    <span class="pda">{percent(p.active)}</span>
  </span>
  <Bar value={p.active} color="var(--disk)" height={5} />
  <span class="pdr">{tr("read")} {rate(p.read)} · {tr("write")} {rate(p.write)}</span>
{/snippet}

<Tile
  title={tr("disk")}
  icon="disk"
  color="var(--disk)"
  {span}
  badge={!adv
    ? undefined
    : badgeKind === "active"
      ? `${cur.active.toFixed(0)}% ${lower(tr("active"))}`
      : badgeKind === "temp" && hottest
        ? celsius(hottest.tempC as number)
        : undefined}
  badgeTone={badgeKind === "temp" && hottest
    ? ((tempTone(hottest) || "ok") as "ok" | "warn" | "bad")
    : cur.active > 90
      ? "bad"
      : cur.active > 70
        ? "warn"
        : "neutral"}
  badgeTip={badgeKind === "temp" && hottest ? `${tr("driveTemp")} · ${modelOf(hottest)}` : undefined}
  badgePick={caps.quick("disk", "badge")}
>
  {#if tabsOn}
    {#if phys.length <= MAX_TABS}
      <div class="seg" role="group" aria-label={tr("pickDiskTip")}>
        <button class:on={!sel} title={tr("allDisksTip")} onclick={() => (chosen = null)}>{tr("allDisks")}</button>
        {#each phys as p (p.index)}
          <button class:on={sel?.index === p.index} title={diskFull(p)} onclick={() => (chosen = p.index)}>
            {diskTab(p)}
          </button>
        {/each}
      </div>
    {:else}
      <OptionMenu pick={diskPick} align="start">
        {#snippet trigger({ toggle, open })}
          <button type="button" class="drop" aria-haspopup="menu" aria-expanded={open} title={diskPick.tip} onclick={toggle}>
            <span>{sel ? diskFull(sel) : tr("allDisks")}</span><Icon name="down" size={10} />
          </button>
        {/snippet}
      </OptionMenu>
    {/if}
  {/if}

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
    <ChartStack tile="disk" defs={CHARTS} height={56} who={sel ? undefined : "disk"} />
    {#if kvShown}
      <div class="divider"></div>
      <div class="kv">
        {#if show("active")}<Kv label={tr("active")} value={percent(cur.active)} title={sum} />{/if}
        {#if show("read")}<Kv label={tr("read")} value={rate(cur.read)} title={sum} />{/if}
        {#if show("write")}<Kv label={tr("write")} value={rate(cur.write)} title={sum} />{/if}
        {#if show("models")}
          {#each models as p, i (i)}
            <Kv
              label={p.media || tr("drives")}
              value={[p.model, namesSize(p.model) ? "" : size(p.size), p.bus].filter(Boolean).join(" · ")}
              wrap
            />
          {/each}
        {/if}
      </div>
    {/if}
    {#if show("perDisk") && !sel && phys.length >= 2}
      <div class="divider"></div>
      <span class="sub">{tr("perDisk")}</span>
      {#each phys as p (p.index)}
        {#if tabsOn}
          <button type="button" class="pd" title={`${diskFull(p)}\n${tr("diskOpenTip")}`} onclick={() => (chosen = p.index)}>
            {@render pdRow(p)}
          </button>
        {:else}
          <div class="pd" title={diskFull(p)}>{@render pdRow(p)}</div>
        {/if}
      {/each}
    {/if}
    {#if show("health") && shownHealth.length}
      <div class="divider"></div>
      <span class="sub">{tr("diskHealth")}</span>
      {#each shownHealth as h (h.index)}
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
  /* many disks: one button that opens the list (OptionMenu) */
  .drop {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    border: 0;
    background: transparent;
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    color: var(--text);
    padding: 5px 10px;
    border-radius: 10px;
    box-shadow: var(--raised-sm);
    cursor: pointer;
  }
  .drop span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .drop:hover,
  .drop[aria-expanded="true"] {
    box-shadow: var(--inset-sm);
  }
  /* one line per physical disk: name and load, a bar, then what it reads and writes */
  .pd {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    border: 0;
    background: transparent;
    font: inherit;
    color: inherit;
    text-align: left;
    padding: 2px 0;
    border-radius: 8px;
  }
  button.pd {
    cursor: pointer;
  }
  button.pd:hover,
  button.pd:focus-visible {
    outline: none;
    box-shadow: var(--inset-sm);
  }
  .pdh {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
    font-size: 11.5px;
    min-width: 0;
  }
  .pdn {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pdn b {
    font-weight: 650;
  }
  .pda {
    flex: none;
    font-weight: 600;
  }
  .pdr {
    font-size: 10.5px;
    color: var(--muted);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
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
