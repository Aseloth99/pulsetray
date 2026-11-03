<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import Tile from "../Tile.svelte";
  import Spark from "../Spark.svelte";
  import Icon from "../Icon.svelte";
  import { lower, tr } from "../tr";
  import { clock, mmss } from "../format";
  import { caps } from "../caps.svelte";
  import { settings } from "../settings.svelte";
  import { store, type Metrics } from "../metrics.svelte";
  import { tip } from "../tipstate.svelte";

  let { adv, span }: { m: Metrics; adv: boolean; span: number } = $props();

  const PRESETS: Record<string, string> = {
    "1.1.1.1": "Cloudflare",
    "1.0.0.1": "Cloudflare",
    "8.8.8.8": "Google",
    "8.8.4.4": "Google",
    "9.9.9.9": "Quad9",
  };
  const label = (t: string) => (t === "gateway" ? tr("gateway") : (PRESETS[t] ?? t));

  /** Everything the tile shows about one target, derived from its sample history. */
  function stats(hist: (number | null)[]) {
    const ok = hist.filter((v): v is number => v !== null);
    const last = hist.length ? hist[hist.length - 1] : undefined;
    let jitter = 0;
    let pairs = 0;
    for (let i = 1; i < hist.length; i++) {
      const a = hist[i - 1];
      const b = hist[i];
      if (a !== null && b !== null) {
        jitter += Math.abs(b - a);
        pairs++;
      }
    }
    // Each graph gets its own scale; a timeout is drawn as a spike to the ceiling so it can't hide as "0 ms".
    const ceil = Math.max(20, (ok.length ? Math.max(...ok) : 0) * 1.4);
    return {
      last,
      avg: ok.length ? ok.reduce((x, y) => x + y, 0) / ok.length : null,
      min: ok.length ? Math.min(...ok) : null,
      max: ok.length ? Math.max(...ok) : null,
      jitter: pairs ? jitter / pairs : null,
      loss: hist.length ? ((hist.length - ok.length) / hist.length) * 100 : 0,
      series: hist.map((v) => v ?? ceil),
      ceil,
      samples: hist.length,
    };
  }

  const rows = $derived(
    settings.pingTargets.map((t) => {
      const e = store.ping[t];
      return { target: t, ip: e?.ip ?? null, ...stats(e?.hist ?? []) };
    }),
  );
  const hasData = $derived(rows.some((r) => r.samples > 0));

  const ms = (v: number | null | undefined) =>
    v === null || v === undefined ? "—" : v < 1 ? "<1" : v.toFixed(v < 10 ? 1 : 0);
  const color = (v: number | null | undefined) =>
    v === undefined
      ? "var(--muted)"
      : v === null
        ? "var(--bad-text)"
        : v < 50
          ? "var(--ok-text)"
          : v < 120
            ? "var(--warn-text)"
            : "var(--bad-text)";

  // ---- test control
  const st = $derived(store.pingStatus);
  /** Continuous mode replaces the timed test: no countdown, no start/stop button while it is on. */
  const continuous = $derived(settings.pingContinuous);
  const live = $derived(!!st?.running || (continuous && !st?.paused));
  /** Graphs only exist once there is something to show: a test is running, continuous is on, or an old run left data. */
  const showRows = $derived(hasData || !!st?.running || continuous);
  let menuOpen = $state(false);
  let notice = $state("");

  async function start(secs: number | null) {
    menuOpen = false;
    notice = "";
    try {
      await invoke("ping_start", { secs });
    } catch (e) {
      notice = String(e) === "fullscreen" ? tr("fullscreenBlocked") : String(e);
    }
  }
  const stop = () => invoke("ping_stop");

  /** Clicks anywhere outside the little menu close it. */
  function closeMenu(e: MouseEvent) {
    if (menuOpen && !(e.target as HTMLElement).closest("[data-ping-menu]")) menuOpen = false;
  }

  const planned = (secs: number | null | undefined) =>
    secs == null ? lower(tr("unlimited")) : `${Math.round(secs / 60)} ${tr("minShort")}`;
  const statusLine = $derived.by(() => {
    if (!st) return "";
    if (continuous) return tr("continuousOn");
    if (st.running) {
      return st.unlimited
        ? `${tr("testRunning")} · ${lower(tr("unlimited"))}`
        : `${tr("testRunning")} · ${mmss(st.remainingSecs ?? 0)} ${tr("remaining")}`;
    }
    if (st.lastTestAt) return `${tr("lastTest")}: ${clock(st.lastTestAt, settings.lang)} · ${planned(st.lastTestPlannedSecs)}`;
    return tr("notTested");
  });
  const warning = $derived(
    notice ||
      (st && !st.running && !continuous && st.lastStopReason === "fullscreen" ? tr("stoppedFullscreen") : "") ||
      (st?.paused ? tr("pausedFullscreen") : ""),
  );

  // ---- Settings > Modules > Customize (advanced view): rows and the header badge
  const show = (row: string) => !adv || settings.shown("ping", row);
  const mean = (vs: (number | null | undefined)[]) => {
    const a = vs.filter((v): v is number => typeof v === "number");
    return a.length ? a.reduce((x, y) => x + y, 0) / a.length : null;
  };
  /** Over the targets that have data: average / latest latency, jitter or packet loss. */
  const badgeInfo = $derived.by((): { text?: string; tone: "ok" | "warn" | "bad" | "neutral" } => {
    const kind = settings.badgeOf("ping");
    const measured = rows.filter((r) => r.samples > 0);
    if (!adv || kind === "none" || !measured.length) return { tone: "neutral" };
    const tone = (v: number, warn: number, bad: number) => (v < warn ? "ok" : v < bad ? "warn" : "bad");
    if (kind === "loss") {
      const v = mean(measured.map((r) => r.loss)) ?? 0;
      return { text: `${lower(tr("loss"))} ${v.toFixed(0)}%`, tone: v === 0 ? "ok" : v < 5 ? "warn" : "bad" };
    }
    if (kind === "jitter") {
      const v = mean(measured.map((r) => r.jitter));
      return v === null ? { tone: "neutral" } : { text: `${lower(tr("jitter"))} ${ms(v)} ms`, tone: tone(v, 10, 30) };
    }
    if (kind === "latest") {
      const v = mean(measured.map((r) => r.last));
      if (v === null) return { text: tr("timeout"), tone: "bad" };
      return { text: `${tr("latestShort")} ${ms(v)} ms`, tone: tone(v, 50, 120) };
    }
    const v = mean(measured.map((r) => r.avg));
    return v === null ? { tone: "neutral" } : { text: `${lower(tr("avg"))} ${ms(v)} ms`, tone: tone(v, 50, 120) };
  });
</script>

<svelte:window onclick={closeMenu} />

{#snippet info()}
  <button class="icon-btn sm" aria-label={tr("pingInfo")} use:tip={tr("pingInfo")}>
    <Icon name="info" size={14} />
  </button>
{/snippet}

<Tile
  title={tr("ping")}
  icon="ping"
  color="var(--net)"
  {span}
  badge={badgeInfo.text}
  badgeTone={badgeInfo.tone}
  badgePick={caps.quick("ping", "badge")}
>
  {#snippet actions()}
    <span class="acts" data-ping-menu>
      {#if adv}{@render info()}{/if}
      {#if continuous}
        <!-- measuring already: nothing to start or stop here, the switch below is the control -->
      {:else if st?.running}
        <button class="icon-btn sm live" aria-label={tr("pingStop")} use:tip={tr("pingStop")} onclick={stop}>
          <Icon name="stop" size={13} />
        </button>
      {:else}
        <button
          class="icon-btn sm"
          aria-label={tr("pingStart")}
          use:tip={menuOpen ? "" : tr("pingStart")}
          onclick={() => (menuOpen = !menuOpen)}
        >
          <Icon name="play" size={13} />
        </button>
      {/if}
      {#if menuOpen && !continuous}
        <div class="menu" role="menu">
          <button role="menuitem" onclick={() => start(180)}>{tr("min3")}</button>
          <button role="menuitem" onclick={() => start(null)}>{tr("unlimited")}</button>
        </div>
      {/if}
    </span>
  {/snippet}

  <div class="status" class:live>
    <span>{statusLine}</span>
    {#if !adv}{@render info()}{/if}
  </div>
  {#if warning}<p class="warn">{warning}</p>{/if}

  {#if adv}
    <div class="cont">
      <span class="lbl" use:tip={tr("continuousTip")}>{tr("continuous")}</span>
      <button
        class="switch"
        class:on={settings.pingContinuous}
        role="switch"
        aria-checked={settings.pingContinuous}
        aria-label={tr("continuous")}
        use:tip={tr("continuousTip")}
        onclick={() => {
          settings.pingContinuous = !settings.pingContinuous;
          menuOpen = false;
          notice = "";
          settings.save();
        }}><i></i></button
      >
    </div>
  {/if}

  {#each showRows ? rows : [] as r (r.target)}
    <div class="blk">
      <div class="head">
        <span class="name" title={r.ip ? `${r.target} (${r.ip})` : r.target}>{label(r.target)}</span>
        <b style="color:{color(r.last)}">
          {#if r.last === undefined}—{:else if r.last === null}{tr("timeout")}{:else}{ms(r.last)}<small> ms</small>{/if}
        </b>
      </div>
      {#if show("chart")}<Spark data={r.series} max={r.ceil} color="var(--net)" height={adv ? 34 : 18} />{/if}
      {#if adv && hasData && show("stats")}
        <div class="stats">
          <span><i>{tr("avg")}</i>{ms(r.avg)}</span>
          <span><i>{tr("min")}</i>{ms(r.min)}</span>
          <span><i>{tr("max")}</i>{ms(r.max)}</span>
          <span><i>{tr("jitter")}</i>{ms(r.jitter)}</span>
          <span><i>{tr("loss")}</i>{r.loss.toFixed(0)}%</span>
        </div>
      {/if}
      {#if adv && hasData && show("ip") && r.ip && r.target !== "gateway" && r.ip !== r.target}
        <span class="sub ip">{r.target} → {r.ip}</span>
      {/if}
    </div>
  {/each}
</Tile>

<style>
  .acts {
    position: relative;
    display: flex;
    align-items: center;
    gap: 5px;
    flex: none;
  }
  .live {
    color: var(--net);
  }
  .menu {
    position: absolute;
    top: 32px;
    right: 0;
    z-index: 30;
    min-width: 120px;
    padding: 5px;
    border-radius: 14px;
    background: var(--bg);
    box-shadow:
      var(--raised),
      0 0 0 1px color-mix(in srgb, var(--lo) 55%, transparent);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .menu button {
    text-align: left;
    padding: 7px 11px;
    border-radius: 10px;
    font-size: 12px;
    color: var(--text);
  }
  .menu button:hover {
    box-shadow: var(--inset-sm);
  }
  .status {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 6px;
    font-size: 10.5px;
    color: var(--muted);
    line-height: 1.3;
  }
  .status.live {
    color: var(--net-text);
    font-weight: 600;
  }
  .warn {
    font-size: 10.5px;
    line-height: 1.3;
    color: var(--warn-text);
  }
  .cont {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: 12px;
  }
  .lbl {
    cursor: help;
    border-bottom: 1px dotted var(--muted);
  }
  .blk {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .blk + .blk {
    padding-top: 8px;
    border-top: 1px solid color-mix(in srgb, var(--lo) 55%, transparent);
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
    font-size: 12px;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  b {
    font-weight: 650;
    flex: none;
  }
  small {
    font-size: 10px;
    font-weight: 500;
    color: var(--muted);
  }
  .stats {
    display: flex;
    justify-content: space-between;
    gap: 6px;
    font-size: 11.5px;
    font-weight: 600;
  }
  .stats span {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
  }
  .stats i {
    font-style: normal;
    font-weight: 400;
    font-size: 10px;
    color: var(--muted);
  }
  .ip {
    font-size: 10.5px;
  }
</style>
