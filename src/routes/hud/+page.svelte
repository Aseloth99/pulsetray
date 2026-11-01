<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import "../../app.css";
  import "../../glass.css";
  import { tr } from "$lib/tr";
  import {
    settings,
    HUD_ITEMS,
    HUD_MIN_OPACITY,
    STYLE_MODES,
    applyStyle,
    type HudSettings,
    type HudItem,
    type StyleMode,
    type ThemeMode,
  } from "$lib/settings.svelte";
  import type { Lang } from "$lib/i18n";
  import { primaryGpu, type Metrics } from "$lib/metrics.svelte";
  import { rate, celsius } from "$lib/format";

  /**
   * Desktop mini gauge: a few live numbers in a small always-on-top window (see hud.rs). Its settings come from
   * Rust (`get_hud` / "hud-config"); it never writes the settings itself (only the main window does).
   */
  let cfg = $state<HudSettings>({ ...settings.hud });
  let m = $state<Metrics | null>(null);
  let card = $state<HTMLElement>();

  let systemDark = $state(false);
  const theme = $derived(settings.theme === "system" ? (systemDark ? "dark" : "light") : settings.theme);
  $effect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.lang = settings.lang;
  });
  $effect(() => {
    applyStyle(settings.style);
  });

  /**
   * The card is at least 85 % solid (see hud.rs MIN_OPACITY): more see-through, moving content behind it flickered
   * through, and a light card over a dark game turned mid grey. The light theme's labels use deeper shades.
   */
  const alpha = $derived(Math.min(1, Math.max(HUD_MIN_OPACITY, cfg.opacity)));

  type Row = { id: HudItem; label: string; value: string; value2?: string; pct?: number; color: string; tip?: string };
  const pct = (used: number, total: number) => (total > 0 ? (used / total) * 100 : 0);
  const rows = $derived.by((): Row[] => {
    if (!m) return [];
    const out: Row[] = [];
    // always the canonical order (CPU, RAM, GPU, ...), whatever order an older settings file stored
    for (const id of HUD_ITEMS.filter((k) => k !== "fps" && cfg.items.includes(k))) {
      if (id === "cpu") {
        // like the GPU row: the temperature next to the load when there is a sensor
        const temp = m.cpuTempC != null ? ` · ${celsius(m.cpuTempC)}` : "";
        out.push({ id, label: "CPU", value: `${m.cpuUsage.toFixed(0)}%${temp}`, pct: m.cpuUsage, color: "var(--cpu)" });
      } else if (id === "gpu") {
        const g = primaryGpu(m.gpus);
        if (!g) continue;
        const temp = g.sensors?.tempC != null ? ` · ${celsius(g.sensors.tempC)}` : "";
        out.push({ id, label: "GPU", value: `${g.usage.toFixed(0)}%${temp}`, pct: g.usage, color: "var(--gpu)" });
      } else if (id === "ram") {
        const p = pct(m.memUsed, m.memTotal);
        out.push({ id, label: "RAM", value: `${p.toFixed(0)}%`, pct: p, color: "var(--mem)" });
      } else if (id === "net") {
        // two lines: one long line changed the card (and window) width every second
        out.push({ id, label: tr("hudNet"), value: `↓ ${rate(m.netDown)}`, value2: `↑ ${rate(m.netUp)}`, color: "var(--net)" });
      } else if (id === "disk") {
        out.push({ id, label: tr("disk"), value: `${m.diskActive.toFixed(0)}%`, pct: m.diskActive, color: "var(--disk)" });
      }
    }
    return out;
  });

  /**
   * FPS on top, large, coloured by how smooth it is (60 and up green, 30–59 orange, below 30 red): in a game it is
   * the number one looks for first. No bar: frame rates have no top (the refresh rate doesn't cap every game).
   */
  const fpsView = $derived.by(() => {
    if (!m || !cfg.items.includes("fps")) return null;
    const f = m.fps;
    const v = f?.fps ?? null;
    const tone = v == null ? "none" : v >= 60 ? "good" : v >= 30 ? "mid" : "low";
    const side =
      v != null && f?.frameMs != null ? `${f.frameMs.toFixed(1)} ms` : f?.error === "denied" ? tr("fpsNoAccess") : "";
    const tip = !f
      ? ""
      : f.error === "denied"
        ? tr("fpsDenied")
        : f.error
          ? tr("fpsFailed")
          : v != null
            ? [f.app, `${v.toFixed(1)} FPS`, f.frameMs != null ? `${f.frameMs.toFixed(1)} ms` : ""].filter(Boolean).join(" · ")
            : `${f.app ? `${f.app}: ` : ""}${tr("fpsNone")}`;
    return { num: v != null ? String(Math.round(v)) : "—", tone, side, tip };
  });

  /** Tells Rust how big the window must be (the card, nothing around it) — only when it really changed: resizing
   *  the window every second (the card followed the text width) made the gauge cost several times more CPU. */
  let reported = "";
  async function reportSize() {
    await tick();
    if (!card) return;
    const size = `${card.offsetWidth}x${card.offsetHeight}`;
    if (size === reported) return;
    reported = size;
    invoke("set_hud_size", { width: card.offsetWidth, height: card.offsetHeight }).catch(() => {});
  }
  $effect(() => {
    rows.length;
    fpsView === null;
    cfg.items.length;
    settings.lang;
    reportSize();
  });

  function onDown(e: MouseEvent) {
    if (e.button !== 0 || cfg.locked) return;
    getCurrentWindow()
      .startDragging()
      .catch(() => {});
  }

  onMount(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    systemDark = mq.matches;
    const onMq = (e: MediaQueryListEvent) => (systemDark = e.matches);
    mq.addEventListener("change", onMq);
    invoke<HudSettings>("get_hud")
      .then((c) => (cfg = c))
      .catch(() => {});
    invoke<Metrics>("get_metrics")
      .then((v) => (m = v))
      .catch(() => {});
    const unMetrics = listen<Metrics>("metrics", (e) => (m = e.payload));
    const unLook = listen<{ theme: ThemeMode; lang: Lang; style?: StyleMode }>("hud-look", (e) => {
      settings.theme = e.payload.theme;
      settings.langPref = e.payload.lang; // already the language in use (the gauge never saves)
      if (e.payload.style && STYLE_MODES.includes(e.payload.style)) settings.style = e.payload.style;
    });
    const unCfg = listen<HudSettings>("hud-config", (e) => {
      cfg = e.payload;
      settings.refresh(); // theme / language may have changed too
      systemDark = mq.matches;
    });
    return () => {
      mq.removeEventListener("change", onMq);
      unMetrics.then((f) => f());
      unCfg.then((f) => f());
      unLook.then((f) => f());
    };
  });
</script>

<svelte:window oncontextmenu={(e) => e.preventDefault()} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="card"
  class:locked={cfg.locked}
  bind:this={card}
  style="--alpha:{Math.round(alpha * 100)}%"
  onmousedown={onDown}
  title={cfg.locked ? "" : tr("hudDragTip")}
>
  {#if fpsView}
    <div class="fps {fpsView.tone}" title={fpsView.tip || undefined}>
      <b>{fpsView.num}</b><span class="u">FPS</span>
      {#if fpsView.side}<span class="ms">{fpsView.side}</span>{/if}
    </div>
  {/if}
  {#each rows as r (r.id)}
    <div class="row" title={r.tip || undefined}>
      <span class="k" style="color:var(--k-{r.id})">{r.label}</span>
      <span class="v">{r.value}</span>
      {#if r.value2}<span class="v v2">{r.value2}</span>{/if}
      {#if r.pct != null}
        <span class="bar"><i style="width:{Math.min(100, Math.max(0, r.pct))}%;background:{r.color}"></i></span>
      {/if}
    </div>
  {:else}
    {#if !fpsView}<div class="row"><span class="v">PulseTray</span></div>{/if}
  {/each}
</div>

<style>
  :global(html),
  :global(body) {
    background: transparent !important;
    margin: 0;
    overflow: hidden;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    /* fixed width: the window keeps its size while the numbers change */
    width: 172px;
    padding: 8px 10px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--bg) var(--alpha), transparent);
    border: 1px solid color-mix(in srgb, var(--lo) 40%, transparent);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    cursor: grab;
    user-select: none;
    box-sizing: border-box;
  }
  /* label colours: the plain accents are for graphics; as text on the light card they were 1.8 to 3:1. The card is the (neutral grey)
     page colour over whatever is behind it, in the worst case 85 % of it over black: these stay above 4.5:1 there. */
  .card {
    --k-fps: #105230;
    --k-fps-mid: #7a3706;
    --k-fps-low: #8d1f1b;
    --k-cpu: #1a43a3;
    --k-ram: #5c2ca8;
    --k-gpu: #8f1b44;
    --k-net: #095243;
    --k-disk: #7a3706;
  }
  :global(:root[data-theme="dark"]) .card {
    --k-fps: var(--bat);
    --k-fps-mid: var(--disk);
    --k-fps-low: #ff7b72;
    --k-cpu: var(--cpu);
    --k-ram: var(--mem);
    --k-gpu: var(--gpu);
    --k-net: var(--net);
    --k-disk: var(--disk);
  }
  /*
   * Glassmorphic style (see glass.css): an opaque-like gradient, never below the card's own opacity (--alpha, 85 % and
   * up), with a faint colour glow in two corners, a rim and a light line on top. No backdrop-filter on purpose: this
   * window is transparent, so there is nothing behind it to blur, and the gauge is always on screen (CPU cost).
   */
  :global(:root[data-style="glass"]:not([data-theme="dark"])) .card {
    background:
      radial-gradient(120px 90px at 100% 0%, rgb(130 172 255 / 0.4), transparent 70%),
      radial-gradient(120px 90px at 0% 100%, rgb(110 222 192 / 0.3), transparent 70%),
      linear-gradient(
        160deg,
        color-mix(in srgb, #ffffff var(--alpha), transparent),
        color-mix(in srgb, #e6e6e6 var(--alpha), transparent)
      );
    border-color: rgb(255 255 255 / 0.85);
    box-shadow:
      inset 0 0 0 1px rgb(60 60 60 / 0.2),
      inset 0 1.5px 0 rgb(255 255 255 / 0.95);
  }
  :global(:root[data-style="glass"][data-theme="dark"]) .card {
    background:
      radial-gradient(120px 90px at 100% 0%, rgb(122 165 255 / 0.13), transparent 70%),
      radial-gradient(120px 90px at 0% 100%, rgb(61 220 187 / 0.08), transparent 70%),
      linear-gradient(
        160deg,
        color-mix(in srgb, #323232 var(--alpha), transparent),
        color-mix(in srgb, #1f1f1f var(--alpha), transparent)
      );
    border-color: rgb(255 255 255 / 0.24);
    box-shadow:
      inset 0 1.5px 0 rgb(255 255 255 / 0.2),
      inset 0 0 0 1px rgb(255 255 255 / 0.05);
  }
  .fps {
    display: flex;
    align-items: baseline;
    gap: 5px;
    padding-bottom: 5px;
    margin-bottom: 1px;
    border-bottom: 1px solid color-mix(in srgb, var(--text) 14%, transparent);
    white-space: nowrap;
  }
  .fps:last-child {
    padding-bottom: 0;
    margin-bottom: 0;
    border-bottom: 0;
  }
  .fps b {
    font-size: 24px;
    font-weight: 800;
    line-height: 1;
    letter-spacing: -0.5px;
  }
  .fps .u {
    font-size: 11px;
    font-weight: 800;
  }
  .fps .ms {
    margin-left: auto;
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
  }
  .fps.good {
    color: var(--k-fps);
  }
  .fps.mid {
    color: var(--k-fps-mid);
  }
  .fps.low {
    color: var(--k-fps-low);
  }
  .fps.none {
    color: var(--muted);
  }
  .card.locked {
    cursor: default;
  }
  .row {
    display: grid;
    grid-template-columns: 34px 1fr;
    grid-template-rows: auto auto;
    column-gap: 6px;
    align-items: center;
  }
  .k {
    font-weight: 700;
    font-size: 11px;
  }
  .v {
    font-weight: 650;
    color: var(--text);
    white-space: nowrap;
    text-align: right;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .v2 {
    grid-column: 2;
  }
  .bar {
    grid-column: 1 / 3;
    height: 3px;
    border-radius: 2px;
    /* a visible track: on the dark card the --lo track was invisible, so 86% looked like an arbitrary length */
    background: color-mix(in srgb, var(--text) 14%, transparent);
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    border-radius: 2px;
  }
</style>
