<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import Kv from "./Kv.svelte";
  import DateChart from "./DateChart.svelte";
  import UsageBars from "./UsageBars.svelte";
  import { tr } from "./tr";
  import { chemistryName, wh } from "./format";
  import { settings } from "./settings.svelte";

  /** What the backend reads from Windows' battery report (batteryreport.rs). */
  type History = {
    reportTime: string;
    batteries: { name: string; manufacturer: string; chemistry: string; designMwh: number; fullMwh: number; cycles: number | null }[];
    capacity: { start: string; end: string; designMwh: number; fullMwh: number; cycles: number | null }[];
    usage: { start: string; end: string; activeAcSecs: number; standbyAcSecs: number; activeDcSecs: number; standbyDcSecs: number }[];
    recent: { t: string; ac: boolean; kind: string; chargeMwh: number; fullMwh: number }[];
  };

  let data = $state<History | null>(null);
  let error = $state("");
  let loading = $state(true);

  async function load(force: boolean) {
    loading = true;
    error = "";
    try {
      data = await invoke<History>("battery_history", { force });
    } catch (e) {
      data = null;
      error = String(e);
    }
    loading = false;
  }
  onMount(() => {
    load(false);
  });

  const loc = $derived(settings.lang === "tr" ? "tr-TR" : "en-GB");
  const day = (t: number) => new Date(t).toLocaleDateString(loc, { day: "numeric", month: "short", year: "numeric" });
  const dayShort = (t: number) => new Date(t).toLocaleDateString(loc, { day: "numeric", month: "short" });
  const when = (t: number) =>
    new Date(t).toLocaleString(loc, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", hour12: false });
  const pctText = (v: number) => `${v.toFixed(v < 100 && v % 1 ? 1 : 0)}%`;
  const hours = (h: number) => (h < 1 ? `${Math.round(h * 60)} ${tr("minShort")}` : `${h.toFixed(1)} ${tr("hourShort")}`);
  /** Report dates are local times without a zone ("2026-10-24T08:00:00"): read them as local. */
  const ms = (s: string) => Date.parse(s.replace(/Z$/, ""));

  const total = $derived.by(() => {
    if (!data?.batteries.length) return null;
    const design = data.batteries.reduce((s, b) => s + b.designMwh, 0);
    const full = data.batteries.reduce((s, b) => s + b.fullMwh, 0);
    const cycles = data.batteries.every((b) => b.cycles != null) ? data.batteries.reduce((s, b) => s + (b.cycles ?? 0), 0) : null;
    return { design, full, cycles, health: design ? Math.min(100, (full / design) * 100) : null, info: data.batteries[0] };
  });

  /** Health (full-charge capacity / design) per period, oldest first. */
  const healthPts = $derived.by(() => {
    if (!data) return [];
    const seen = new Set<number>();
    return data.capacity
      .map((c) => ({ t: ms(c.end), v: Math.min(100, (c.fullMwh / c.designMwh) * 100), note: `${wh(c.fullMwh)}${c.cycles ? ` · ${c.cycles} ${tr("cyclesShort")}` : ""}` }))
      .filter((p) => !Number.isNaN(p.t) && !seen.has(p.t) && (seen.add(p.t), true))
      .sort((a, b) => a.t - b.t);
  });
  /** How much health went per year, from the first to the last record (needs a few months of history). */
  const trend = $derived.by(() => {
    const p = healthPts;
    if (p.length < 2) return null;
    const years = (p[p.length - 1].t - p[0].t) / (365.25 * 86400e3);
    return { first: p[0], perYear: years >= 0.25 ? (p[0].v - p[p.length - 1].v) / years : null };
  });
  const healthMin = $derived(Math.max(0, Math.floor((Math.min(100, ...healthPts.map((p) => p.v)) - 5) / 5) * 5));

  /**
   * Screen-on hours per day over the last two weeks. Only daily records count (Windows folds older days into weeks,
   * so a 14-day report can hold fewer days: the title says how many). A day missing between two records had no
   * use and gets an empty bar, so every bar is one calendar day.
   */
  const usageDays = $derived.by(() => {
    if (!data) return [];
    const byDay = new Map<number, { a: number; b: number }>();
    const dayOf = (t: number) => {
      const d = new Date(t);
      return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    };
    for (const u of data.usage) {
      const t = ms(u.start);
      if (Number.isNaN(t) || ms(u.end) - t > 1.5 * 86400e3) continue;
      const k = dayOf(t);
      const v = byDay.get(k) ?? { a: 0, b: 0 };
      v.a += u.activeDcSecs / 3600;
      v.b += u.activeAcSecs / 3600;
      byDay.set(k, v);
    }
    const keys = [...byDay.keys()].sort((x, y) => x - y);
    if (!keys.length) return [];
    const out: { label: string; a: number; b: number }[] = [];
    // walk calendar days (noon steps survive the daylight-saving change), at most the last 14
    for (let t = keys[keys.length - 1]; out.length < 14 && t >= keys[0]; ) {
      const v = byDay.get(t) ?? { a: 0, b: 0 };
      out.unshift({ label: dayShort(t), ...v });
      t = dayOf(t - 12 * 3600e3);
    }
    return out;
  });

  /** Charge level at every state change over the last days, and the stretches on AC. */
  const charge = $derived.by(() => {
    if (!data) return { pts: [], ac: [] };
    const rows = data.recent
      .map((r) => ({ t: ms(r.t), v: Math.min(100, (r.chargeMwh / r.fullMwh) * 100), ac: r.ac }))
      .filter((r) => !Number.isNaN(r.t))
      .sort((a, b) => a.t - b.t);
    const ac: { from: number; to: number }[] = [];
    rows.forEach((r, i) => {
      if (r.ac) ac.push({ from: r.t, to: rows[i + 1]?.t ?? r.t });
    });
    return { pts: rows.map((r) => ({ t: r.t, v: r.v, note: r.ac ? tr("onAc") : tr("onBattery") })), ac };
  });
</script>

{#if loading}
  <section class="card"><p class="sub">{tr("batHistLoading")}</p></section>
{:else if error}
  <section class="card">
    <p class="sub">{error === "no_battery" ? tr("batHistNoBattery") : `${tr("batHistFailed")} ${error}`}</p>
  </section>
{:else if data}
  {#if total}
    <section class="card">
      <h3>{tr("batHistNow")}</h3>
      <div class="kv">
        {#if total.health != null}<Kv label={tr("health")} value={pctText(total.health)} />{/if}
        <Kv label={tr("batHistCapacity")} value="{wh(total.full)} / {wh(total.design)}" />
        {#if total.cycles != null}<Kv label={tr("cycles")} value={String(total.cycles)} />{/if}
        {#if trend}
          <Kv label={tr("batHistFirst")} value="{pctText(trend.first.v)} · {day(trend.first.t)}" />
          {#if trend.perYear != null && trend.perYear > 0.05}
            <Kv label={tr("batHistPerYear")} value="~{pctText(trend.perYear)}" />
          {/if}
        {/if}
        {#if total.info.manufacturer || total.info.chemistry}
          <Kv label={tr("batHistMade")} value={[total.info.manufacturer, chemistryName(total.info.chemistry)].filter(Boolean).join(" · ")} />
        {/if}
      </div>
    </section>
  {/if}

  {#if healthPts.length > 1}
    <section class="card">
      <h3>{tr("batHistHealth")}</h3>
      <p class="sub hint">{tr("batHistHealthHint")}</p>
      <DateChart points={healthPts} color="var(--bat)" max={100} min={healthMin} fmt={pctText} dateFmt={day} />
    </section>
  {/if}

  {#if usageDays.length}
    <section class="card">
      <h3>{tr("batHistUsage").replace("{n}", String(usageDays.length))}</h3>
      <UsageBars days={usageDays} colorA="var(--disk)" colorB="var(--bat)" labelA={tr("onBattery")} labelB={tr("onAc")} fmt={hours} />
    </section>
  {/if}

  {#if charge.pts.length > 1}
    <section class="card">
      <h3>{tr("batHistCharge")}</h3>
      <p class="sub hint">{tr("batHistChargeHint")}</p>
      <DateChart points={charge.pts} shade={charge.ac} color="var(--bat)" max={100} fmt={pctText} dateFmt={when} step />
    </section>
  {/if}

  <section class="card">
    <p class="sub">{tr("batHistSource")}</p>
    <button class="small-btn" onclick={() => load(true)}>{tr("batHistRefresh")}</button>
  </section>
{/if}

<style>
  /* the same cards as the Settings page (its styles are local to it) */
  .card {
    background: var(--bg);
    border-radius: 20px;
    box-shadow: var(--raised);
    padding: 13px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .card h3 {
    font-size: 12px;
    font-weight: 650;
    color: var(--muted);
  }
  .hint {
    margin-top: -6px;
  }
  .kv {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .small-btn {
    align-self: flex-start;
  }
</style>
