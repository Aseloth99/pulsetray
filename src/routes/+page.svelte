<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { emitTo, listen } from "@tauri-apps/api/event";
  import { getVersion } from "@tauri-apps/api/app";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import "../app.css";
  import "../glass.css";
  import Icon from "$lib/Icon.svelte";
  import Tooltip from "$lib/Tooltip.svelte";
  import { tip } from "$lib/tipstate.svelte";
  import CpuTile from "$lib/tiles/CpuTile.svelte";
  import MemoryTile from "$lib/tiles/MemoryTile.svelte";
  import NetworkTile from "$lib/tiles/NetworkTile.svelte";
  import DiskTile from "$lib/tiles/DiskTile.svelte";
  import GpuTile from "$lib/tiles/GpuTile.svelte";
  import BatteryTile from "$lib/tiles/BatteryTile.svelte";
  import ProcessesTile from "$lib/tiles/ProcessesTile.svelte";
  import SystemTile from "$lib/tiles/SystemTile.svelte";
  import PingTile from "$lib/tiles/PingTile.svelte";
  import WifiTile from "$lib/tiles/WifiTile.svelte";
  import type { Key } from "$lib/i18n";
  import { tr } from "$lib/tr";
  import {
    settings,
    SYSTEM_LANG,
    DEFAULT_PING_TARGETS,
    HISTORY_CHOICES,
    TRAY_ICONS,
    HUD_ITEMS,
    DISK_PCTS,
    GPU_TEMPS,
    HOG_PCTS,
    STYLE_MODES,
    applyStyle,
    type AlertSettings,
    type HudItem,
    type TileId,
    type StyleMode,
    type ThemeMode,
    type TrayIcon,
    type ViewMode,
  } from "$lib/settings.svelte";
  import { THEME_MODES, LANG_PREFS, langChoiceLabel } from "$lib/prefs";
  import { store } from "$lib/metrics.svelte";
  import { caps } from "$lib/caps.svelte";
  import BatteryHistory from "$lib/BatteryHistory.svelte";
  import UpdateView from "$lib/UpdateView.svelte";
  import { update } from "$lib/update.svelte";
  import { DETAIL } from "$lib/tiledetail";

  let view = $state<"main" | "settings" | "battery" | "update">("main");
  type Notice = { id: string; text: string; url: string; level: string };
  /** Announcement from macik.com.tr; nothing is shown until the server sends one and until it is dismissed. */
  let notice = $state<Notice | null>(null);
  const noticeShown = $derived(notice !== null && notice.id !== settings.dismissedNotice);
  const SITE = "https://macik.com.tr";
  function dismissNotice() {
    if (!notice) return;
    settings.dismissedNotice = notice.id;
    settings.save();
  }
  let autostart = $state(false);
  let systemDark = $state(false);

  const m = $derived(store.data);
  const adv = $derived(settings.advanced);
  const theme = $derived(settings.theme === "system" ? (systemDark ? "dark" : "light") : settings.theme);

  /**
   * Columns follow the width: as many as fit (1 to 3). Advanced tiles carry more text, so they
   * need more room each. Before the first measurement fall back to the fixed popup widths.
   */
  let mainW = $state(0);
  const cols = $derived.by(() => {
    if (!mainW) return adv ? 2 : 3;
    const content = mainW - 36;
    const minTile = adv ? 320 : 150;
    return Math.min(3, Math.max(1, Math.floor((content + 16) / (minTile + 16))));
  });
  const POPUP_WIDTH = { classic: 564, advanced: 736 };

  /**
   * Whether the hardware behind an optional tile is there (no battery on a desktop, no GPU, no Wi-Fi
   * link). One failed read must not make a tile vanish and the masonry re-balance, so a tile that has
   * been showing stays until its data has been missing for several updates in a row.
   */
  const GONE_AFTER = 4;
  const missing = { battery: 0, gpu: 0, wifi: 0 };
  let present = $state({ battery: false, gpu: false, wifi: false });
  $effect(() => {
    const cur = m;
    if (!cur || view !== "main") return;
    const now = { battery: cur.battery.present, gpu: cur.gpus.length > 0, wifi: cur.wifi != null };
    untrack(() => {
      for (const k of ["battery", "gpu", "wifi"] as const) {
        if (now[k]) {
          missing[k] = 0;
          if (!present[k]) present[k] = true;
        } else if (present[k] && ++missing[k] >= GONE_AFTER) {
          present[k] = false;
        }
      }
    });
  });

  // Which tile options this PC can show (Settings greys out the rest). GPU sensors are only read while the GPU tile is on
  // screen; everything else is measured all the time, so it keeps being judged while Settings is open.
  $effect(() => {
    const cur = m;
    if (!cur) return;
    const gpuMeasured = view === "main" && settings.tiles.some((t) => t.id === "gpu" && t.visible);
    untrack(() => caps.observe(cur, gpuMeasured));
  });

  /** Modules whose hardware isn't there (only known once samples came in on the main view). */
  const absentKey: Partial<Record<TileId, Key>> = { battery: "absentBattery", wifi: "absentWifi", gpu: "absentGpu" };
  let sampled = $state(false);
  $effect(() => {
    if (m && view === "main" && !sampled) setTimeout(() => (sampled = true), 3000);
  });
  function absent(id: TileId): Key | undefined {
    const key = absentKey[id];
    return key && sampled && !present[id as "battery" | "gpu" | "wifi"] ? key : undefined;
  }

  /** Tiles that currently have something to show. */
  const visibleTiles = $derived(
    settings.tiles
      .filter((x) => {
        if (!x.visible) return false;
        if (x.id === "battery" || x.id === "gpu" || x.id === "wifi") return present[x.id];
        return true;
      })
      .map((x) => x.id),
  );

  /**
   * Masonry: every tile is as tall as its content and goes into the currently shortest column, so
   * nothing is stretched and no gaps are left. Heights are estimated at first, then measured once per
   * layout (columns / mode / tile set / language) and frozen, so tiles never jump columns while the
   * numbers tick.
   */
  const EST: Record<TileId, [number, number]> = {
    cpu: [235, 340],
    memory: [265, 420],
    gpu: [255, 340],
    network: [265, 430],
    disk: [225, 245],
    battery: [250, 375],
    ping: [190, 330],
    processes: [200, 210],
    wifi: [180, 190],
    system: [170, 230],
  };
  let measured = $state<Record<string, number>>({});
  /** Bumped every time the popup is shown: the lists inside the tiles may have changed in the meantime. */
  let shownCount = $state(0);
  /** Tiles whose height depends on data that arrives a moment after start-up (process list, hardware facts). */
  const settled = $derived(`${(m?.topCpu.length ?? 0) > 0}|${store.hw != null}|${m?.disks.length ?? 0}`);
  /** Customised rows / charts change tile heights too. */
  const detailKey = $derived(JSON.stringify(settings.details));
  const layoutKey = $derived(`${cols}|${adv}|${visibleTiles.join(",")}|${settings.lang}|${settled}|${shownCount}|${detailKey}`);
  let measuredKey = "";

  const columns = $derived.by(() => {
    const out: TileId[][] = Array.from({ length: cols }, () => []);
    const h = new Array<number>(cols).fill(0);
    for (const id of visibleTiles) {
      let k = 0;
      for (let i = 1; i < cols; i++) if (h[i] < h[k]) k = i;
      out[k].push(id);
      h[k] += (measured[id] ?? EST[id][adv ? 1 : 0]) + 16;
    }
    return out;
  });

  /**
   * The app list fills what its column has left below the tallest one with more apps (up to 10), instead of
   * leaving an empty strip. Worked out from the measured heights, which never include these extra rows, so the
   * layout itself doesn't move.
   */
  let procRowH = $state(0);
  const procExtra = $derived.by(() => {
    const k = columns.findIndex((c) => c.includes("processes"));
    if (k < 0 || !procRowH || measured.processes == null) return 0;
    const hs = columns.map((c) => c.reduce((s, id) => s + (measured[id] ?? EST[id][adv ? 1 : 0]) + 16, 0));
    return Math.max(0, Math.floor((Math.max(...hs) - hs[k] - 4) / procRowH));
  });

  /** Height of the bars between the header and the tiles (announcement, "Update · vX" pill). */
  function noticeBarsH(): number {
    let sum = 0;
    document.querySelectorAll<HTMLElement>(".notice").forEach((el) => (sum += el.offsetHeight + 6));
    return sum;
  }
  /** Everything in the popup except the tiles themselves: header, notice line, paddings. */
  function chromeH(): number {
    const head = document.querySelector<HTMLElement>("header.top");
    const main = document.querySelector<HTMLElement>("main");
    if (!head || !main) return 130;
    const cs = getComputedStyle(main);
    const bar = noticeBarsH();
    return head.offsetHeight + bar + parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom) + 28;
  }
  /** Height the tiles need in `n` columns (same shortest-column rule as the masonry, from measured heights). */
  function heightIn(n: number): number {
    const h = new Array<number>(n).fill(0);
    for (const id of visibleTiles) {
      let k = 0;
      for (let i = 1; i < n; i++) if (h[i] < h[k]) k = i;
      h[k] += (measured[id] ?? EST[id][adv ? 1 : 0]) + 16;
    }
    return Math.max(...h) - 16 + chromeH();
  }
  /**
   * Advanced mode normally has two columns. When two columns would be taller than a comfortable height
   * (MAX_POPUP_H) and the screen is wide enough, the popup goes three columns wide instead.
   */
  const WIDE_WIDTH = 1064;
  const popupW = $derived.by(() => {
    const mode = adv ? "advanced" : "classic";
    if (!adv || screen.availWidth < WIDE_WIDTH + 40 || view !== "main") return POPUP_WIDTH[mode];
    const cap = Math.min(MAX_POPUP_H, screen.availHeight - 16);
    return heightIn(2) > cap ? WIDE_WIDTH : POPUP_WIDTH[mode];
  });

  $effect(() => {
    const key = layoutKey;
    if (view !== "main" || !m || key === measuredKey) return;
    tick().then(() =>
      requestAnimationFrame(() => {
        const next: Record<string, number> = {};
        const gapAfter = (el: HTMLElement) => parseFloat(getComputedStyle(el.parentElement!).rowGap) || 0;
        document.querySelectorAll<HTMLElement>("[data-tile]").forEach((el) => {
          let h = el.offsetHeight;
          // The app list's extra rows only fill leftover room: they are not part of the tile's own height.
          el.querySelectorAll<HTMLElement>("[data-extra]").forEach((r) => (h -= r.offsetHeight + gapAfter(r)));
          next[el.dataset.tile!] = h;
        });
        const row = document.querySelector<HTMLElement>("[data-tile=processes] .row");
        if (row) procRowH = row.offsetHeight + gapAfter(row);
        measuredKey = key;
        measured = next;
      }),
    );
  });

  type Edge = "North" | "South" | "East" | "West" | "NorthEast" | "NorthWest" | "SouthEast" | "SouthWest";
  const edges: { dir: Edge; cls: string }[] = [
    { dir: "North", cls: "n" },
    { dir: "South", cls: "s" },
    { dir: "East", cls: "e" },
    { dir: "West", cls: "w" },
    { dir: "NorthEast", cls: "ne" },
    { dir: "NorthWest", cls: "nw" },
    { dir: "SouthEast", cls: "se" },
    { dir: "SouthWest", cls: "sw" },
  ];
  /** True from the moment a grip is pressed until the mouse is next seen over the page (the drag has ended). */
  let userResizing = false;
  const startResize = (dir: Edge) => (e: MouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    userResizing = true;
    // The drag briefly takes focus from the popup; tell Rust so it doesn't hide it as "clicked elsewhere".
    invoke("set_resizing", { on: true }).catch(() => {});
    getCurrentWindow().startResizeDragging(dir);
  };
  function endResize() {
    if (!userResizing) return;
    userResizing = false;
    invoke("set_resizing", { on: false }).catch(() => {});
  }
  /** A pinned popup has no remembered size of its own; this says whether it currently hugs its content. */
  let pinnedFitted = $state(false);
  /** Measured, not remembered: whatever way the window got its size (grip, window border, pinned default). */
  let winW = $state(0);
  let winH = $state(0);
  let needH = $state<number | null>(null);
  /** The "fit to content" button shows whenever the popup is not exactly as wide/tall as its tiles want. */
  const showFit = $derived(
    view === "main" &&
      needH !== null &&
      (Math.abs(winH - needH) > 8 || Math.abs(winW - Math.min(popupW, screen.availWidth - 16)) > 8),
  );
  function refreshSizes() {
    winW = window.innerWidth;
    winH = window.innerHeight;
    needH = measureNeed();
  }
  /** Fit button (or double-click on an edge): forget the remembered size and hug the content again. */
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const fitContent = async () => {
    const mode = adv ? "advanced" : "classic";
    settings.sizes[mode] = null;
    settings.save();
    pinnedFitted = true;
    try {
      await invoke("set_custom_size", { custom: false });
      const width = Math.min(popupW, screen.availWidth - 16);
      await invoke("set_popup_width", { width: popupW });
      if (settings.pinned) {
        // The tiles reflow when the width changes, so the height is measured after the new width is in place.
        await invoke("resize_popup", { width, height: window.innerHeight });
        for (let i = 0; i < 2; i++) {
          await sleep(250);
          const need = measureNeed();
          if (need !== null) await invoke("resize_popup", { width, height: need });
        }
      } else {
        await sleep(250);
        fit();
      }
    } catch {
      /* the window is gone or the call failed: nothing to fit */
    }
    refreshSizes();
  };

  /**
   * The popup is exactly as tall as its tiles (no empty strip at the bottom); a pinned popup is sized by
   * the user, and Settings keeps the full height. Rust clamps the value to the screen.
   * MAX_POPUP_H only decides two or three columns; the height itself may grow up to the screen (MAX_FIT_H is
   * Rust's own limit in `set_popup_height`), so the tiles are never cut off at the bottom when they fit.
   */
  const MAX_POPUP_H = 880;
  const MAX_FIT_H = 1400;
  let masonryEl = $state<HTMLElement>();
  /** Height the popup needs to show all tiles, or null when the main view isn't laid out. */
  function measureNeed(): number | null {
    if (view !== "main" || !masonryEl) return null;
    const head = document.querySelector<HTMLElement>("header.top");
    const main = document.querySelector<HTMLElement>("main");
    if (!head || !main) return null;
    const cs = getComputedStyle(main);
    const bar = noticeBarsH();
    const need = head.offsetHeight + bar + parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom) + masonryEl.offsetHeight + 28;
    // Only content taller than the screen scrolls.
    return Math.min(Math.ceil(need), MAX_FIT_H, screen.availHeight - 16);
  }
  function fit() {
    const need = measureNeed();
    if (need === null) return;
    if (!settings.pinned) invoke("set_popup_height", { height: need });
    else if (pinnedFitted && settings.sizes[adv ? "advanced" : "classic"] == null) {
      // A pinned popup that hugs its content keeps doing so when the content grows (e.g. the app list arrives).
      invoke("resize_popup", { width: Math.min(popupW, screen.availWidth - 16), height: need });
    }
    // The window follows a moment later; measure again once it has.
    setTimeout(refreshSizes, 250);
  }
  /**
   * After a show Rust keeps the window invisible until this has run (`popup_settled` in lib.rs): wait for the layout
   * measurement and the reflow it causes, give Rust the final height, then say "ready". Without an answer Rust reveals
   * the window by itself after 250 ms, so the popup never visibly changes size right after it appears.
   */
  async function settleAfterShow() {
    const frame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));
    const t0 = performance.now();
    await tick();
    // Done once the measurement has run and the needed height stayed the same for a few frames in a row (the
    // reflow and its ResizeObserver callback come after the measurement); never longer than 180 ms.
    let need: number | null = null;
    let stable = 0;
    while (performance.now() - t0 < 180) {
      await frame();
      const now = measuredKey === layoutKey ? measureNeed() : null;
      stable = now !== null && now === need ? stable + 1 : 0;
      need = now;
      if (stable >= 3) break;
    }
    if (need !== null && !settings.pinned) await invoke("set_popup_height", { height: need }).catch(() => {});
    invoke("popup_settled").catch(() => {});
  }
  $effect(() => {
    if (view === "settings" || view === "battery") invoke("set_popup_height", { height: 744 });
    if (view !== "main" || !masonryEl) return;
    const ro = new ResizeObserver(fit);
    ro.observe(masonryEl);
    fit();
    return () => ro.disconnect();
  });

  // The announcement line and the update pill change the content height: re-fit when they appear or go away.
  $effect(() => {
    noticeShown;
    update.offer;
    tick().then(fit);
  });

  let version = $state("");
  /** Result of the last "Check" in Settings (an offer shows as the "Update · vX" button instead, see the template). */
  type CheckState = "idle" | "checking" | "ok" | "failed" | "dev";
  let updateState = $state<CheckState>("idle");
  let checkBusy = $state(false);

  // The update screen is as tall as its content (the notes box scrolls inside it).
  const UPDATE_MIN_H = 300;
  let updEl = $state<HTMLElement>();
  function fitUpdate() {
    const head = document.querySelector<HTMLElement>("header.top");
    const main = document.querySelector<HTMLElement>("main");
    if (!head || !main || !updEl) return;
    const cs = getComputedStyle(main);
    const need = head.offsetHeight + parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom) + updEl.offsetHeight + 28;
    invoke("set_popup_height", { height: Math.min(Math.max(Math.ceil(need), UPDATE_MIN_H), MAX_FIT_H, screen.availHeight - 16) });
  }
  $effect(() => {
    if (view !== "update" || !updEl) return;
    const ro = new ResizeObserver(fitUpdate);
    ro.observe(updEl);
    fitUpdate();
    return () => ro.disconnect();
  });
  /** Back from the update screen. Leaving while a download runs stops it (a download nobody sees must not end in a restart). */
  function leaveUpdate() {
    if (update.phase.state === "downloading") update.cancel();
    view = "main";
  }

  $effect(() => {
    invoke("set_pinned", { pinned: settings.pinned });
  });

  // Only measure what is on screen (the process list, GPU counters, battery IOCTLs and pings cost CPU).
  $effect(() => {
    // The live preview in "Customize" shows the tile too, so its data is measured while that panel is open.
    const on = (id: TileId) =>
      (view === "main" && settings.tiles.some((t) => t.id === id && t.visible)) || (view === "settings" && openDetail === id);
    invoke("set_features", {
      processes: on("processes"),
      gpu: on("gpu"),
      battery: on("battery"),
      wifi: on("wifi"),
      ping: settings.tiles.some((t) => t.id === "ping" && t.visible),
    });
  });

  $effect(() => {
    invoke("set_ping_targets", { targets: $state.snapshot(settings.pingTargets) });
  });

  $effect(() => {
    invoke("set_tray_icon", { mode: settings.trayIcon }).catch(() => {});
  });

  // ---- Settings > Alerts (the backend watches the samples and sends Windows notifications)
  $effect(() => {
    invoke("set_alerts", { config: { ...$state.snapshot(settings.alerts), lang: settings.lang } }).catch(() => {});
  });
  type AlertRow = { id: "disk" | "memory" | "temp" | "health" | "hog"; label: Key; sub: Key; choice?: Choice };
  type Choice = { key: "diskPct" | "gpuTemp" | "hogPct"; label: Key; options: number[]; text: (v: number) => string };
  const pctText = (v: number) => (settings.lang === "tr" ? `%${v}` : `${v}%`);
  const ALERT_ROWS: AlertRow[] = [
    { id: "disk", label: "alertDisk", sub: "alertDiskSub", choice: { key: "diskPct", label: "alertDiskPct", options: DISK_PCTS, text: pctText } },
    { id: "memory", label: "alertMemory", sub: "alertMemorySub" },
    { id: "temp", label: "alertTemp", sub: "alertTempSub", choice: { key: "gpuTemp", label: "alertGpuTemp", options: GPU_TEMPS, text: (v) => `${v} °C` } },
    { id: "health", label: "alertHealth", sub: "alertHealthSub" },
    { id: "hog", label: "alertHog", sub: "alertHogSub", choice: { key: "hogPct", label: "alertHogPct", options: HOG_PCTS, text: pctText } },
  ];
  let alertTestMsg = $state<{ text: string; bad: boolean } | null>(null);
  async function testAlert() {
    try {
      await invoke("alert_test", { lang: settings.lang });
      alertTestMsg = { text: tr("alertTestSent"), bad: false };
    } catch (e) {
      alertTestMsg = { text: `${tr("alertTestFailed")} ${String(e)}`, bad: true };
    }
  }

  // Desktop mini gauge: this window owns the settings, Rust shows / places the gauge window (hud.rs).
  $effect(() => {
    invoke("set_hud", { config: $state.snapshot(settings.hud) }).catch(() => {});
  });
  // The same short names the gauge shows (the long tile names didn't fit the narrow popup).
  const hudLabel = (id: HudItem): string =>
    id === "net" ? tr("hudNet") : id === "disk" ? tr("disk") : id.toUpperCase();
  const HUD_OPACITY = [0.85, 0.9, 0.95, 1];
  function setHud(change: Partial<typeof settings.hud>) {
    settings.hud = { ...settings.hud, ...change };
    settings.save();
  }
  function toggleHudItem(id: HudItem) {
    const has = settings.hud.items.includes(id);
    // At least one number stays on the gauge.
    if (has && settings.hud.items.length === 1) return;
    setHud({ items: HUD_ITEMS.filter((k) => (k === id ? !has : settings.hud.items.includes(k))) });
  }

  // A longer chart window needs the older samples the backend kept; a shorter one is cut on the next sample.
  let lastHistorySecs = settings.historySecs;
  $effect(() => {
    const secs = settings.historySecs;
    if (secs === lastHistorySecs) return;
    lastHistorySecs = secs;
    untrack(() => store.loadHistory(true).catch(() => {}));
  });

  $effect(() => {
    invoke("set_ping_continuous", { on: settings.pingContinuous });
  });

  // Ctrl+Alt+P. `false` back from Rust = another app already owns the combination.
  let hotkeyOk = $state(true);
  $effect(() => {
    invoke<boolean>("set_hotkey_enabled", { enabled: settings.hotkey })
      .then((registered) => (hotkeyOk = registered || !settings.hotkey))
      .catch(() => (hotkeyOk = false));
  });

  /** The latency targets only matter while the latency module is on. */
  // The latency targets card: the latency tile is on in either view.
  const pingOn = $derived(
    [...settings.tileSets.classic, ...settings.tileSets.advanced].some((t) => t.id === "ping" && t.visible),
  );
  /** Settings > Modules: which view's set is being edited; opens on the view in use. */
  let modTab = $state<ViewMode>("classic");
  $effect(() => {
    if (view === "settings") untrack(() => (modTab = settings.advanced ? "advanced" : "classic"));
  });
  let pingInput = $state("");
  /** Module whose "Customize" panel is open in Settings (one at a time). */
  let openDetail = $state<TileId | null>(null);
  function addTarget() {
    const t = pingInput.trim().toLowerCase();
    if (!t || !/^[a-z0-9.-]+$/.test(t) || settings.pingTargets.includes(t) || settings.pingTargets.length >= 6) return;
    settings.pingTargets = [...settings.pingTargets, t];
    settings.save();
    pingInput = "";
  }
  function removeTarget(t: string) {
    settings.pingTargets = settings.pingTargets.filter((x) => x !== t);
    settings.save();
  }
  function resetTargets() {
    settings.pingTargets = [...DEFAULT_PING_TARGETS];
    settings.save();
  }

  // A size the user dragged to is remembered, separately for classic and advanced (pinned or not).
  let sizeTimer: ReturnType<typeof setTimeout> | undefined;
  function onResize() {
    refreshSizes();
    if (!userResizing) return;
    clearTimeout(sizeTimer);
    sizeTimer = setTimeout(() => {
      settings.sizes[adv ? "advanced" : "classic"] = { w: window.innerWidth, h: window.innerHeight };
      pinnedFitted = false;
      settings.save();
    }, 400);
  }

  // The backend gets both boxes: the daily background check only runs with "Check for Updates Automatically" on, and the
  // automatic install needs that check as well (lib.rs `automatic_install`).
  $effect(() => {
    invoke("set_auto_update", { enabled: settings.autoUpdate });
  });
  $effect(() => {
    invoke("set_auto_check", { enabled: settings.autoCheck });
  });

  /** The check is often instant; keep the spinner for a moment so the click visibly does something. */
  const MIN_CHECK_MS = 1200;
  async function checkNow() {
    if (checkBusy) return;
    checkBusy = true;
    updateState = "checking";
    const started = performance.now();
    let next: CheckState = "failed";
    try {
      const r = await invoke<string>("check_update_now");
      // "up_to_date", "dev" or "offer:<version>" (then the offer itself is read: the button below opens its screen)
      next = r === "dev" ? "dev" : "ok";
      if (r.startsWith("offer:")) await update.refresh();
    } catch {
      next = "failed";
    }
    const left = MIN_CHECK_MS - (performance.now() - started);
    if (left > 0) await new Promise((done) => setTimeout(done, left));
    updateState = next;
    checkBusy = false;
  }

  const updateText = $derived(
    {
      idle: "",
      checking: tr("checking"),
      ok: "✓  " + tr("upToDate"),
      failed: "✕  " + tr("checkFailed"),
      dev: tr("devBuild"),
    }[updateState],
  );

  $effect(() => {
    const mode = adv ? "advanced" : "classic";
    const s = settings.sizes[mode] != null;
    const pinned = settings.pinned;
    const width = popupW; // re-run when the popup should become wider/narrower
    untrack(() => {
      invoke("set_custom_size", { custom: s })
        .then(() => {
          invoke("set_popup_width", { width });
          const size = settings.sizes[mode];
          if (size) invoke("resize_popup", { width: size.w, height: size.h });
          else if (pinned) {
            // Pinned without a remembered size: keep hugging the content if it already does, else the default size.
            const need = pinnedFitted ? measureNeed() : null;
            invoke("resize_popup", { width, height: need ?? 744 });
            if (pinnedFitted) setTimeout(fit, 350);
          } else fit();
        })
        .catch(() => {});
    });
  });

  $effect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.lang = settings.lang;
  });
  // Neumorphic or glass: the attribute app.css / glass.css key on (neumorphic has none).
  $effect(() => {
    applyStyle(settings.style);
  });
  // The mini gauge is its own window: switching the theme, the style or the language here must reach it right away.
  $effect(() => {
    emitTo("hud", "hud-look", { theme: settings.theme, lang: settings.lang, style: settings.style }).catch(() => {});
  });

  // The tray's right-click menu is built by Rust before the UI exists; keep its language in step.
  $effect(() => {
    invoke("set_language", { lang: settings.lang });
  });

  onMount(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    systemDark = mq.matches;
    const onMq = (e: MediaQueryListEvent) => (systemDark = e.matches);
    mq.addEventListener("change", onMq);

    store.start();
    const onShow = () => {
      if (document.visibilityState === "visible") store.loadPing().catch(() => {});
    };
    document.addEventListener("visibilitychange", onShow);
    window.addEventListener("focus", onShow);
    getVersion()
      .then((v) => {
        version = v;
        // Started by an update: the new version says so (a Windows notification; the backend ignores this on any other start).
        invoke("announce_update", { title: "PulseTray", body: tr("updateInstalledToast").replace("{v}", v) }).catch(() => {});
      })
      .catch(() => {});
    if (settings.firstRun) {
      // A fresh install starts at login (Settings turns it off; it is never switched on again after that).
      settings.firstRun = false;
      settings.save();
      setAutostart(true);
    } else {
      invoke<boolean>("autostart_get")
        .then((v) => (autostart = v))
        .catch(() => {});
    }
    const unlisten = listen("open-settings", () => (view = "settings"));
    invoke<Notice | null>("get_notice")
      .then((n) => (notice = n))
      .catch(() => {});
    setTimeout(refreshSizes, 500);
    const unNotice = listen<Notice>("notice", (e) => (notice = e.payload));
    // The update on offer and how it is going (offer, download progress, errors): the store follows the backend's events.
    const stopUpdate = update.start();
    // The tray menu's "Update · vX": the popup opens on the update screen.
    const unOpenUpdate = listen("open-update", () => {
      if (update.offer) view = "update";
    });
    // The tray menu's Theme entry: this window owns the settings, so it records the choice (the effects below then put it
    // on the page, on the mini gauge and, through the saved settings, on the menu itself).
    const unTheme = listen<string>("theme-chosen", (e) => {
      const mode = THEME_MODES.find((t) => t === e.payload);
      if (!mode || settings.theme === mode) return;
      settings.theme = mode;
      settings.save();
    });
    // The tray menu (or Alt+F4 on the gauge) switched the mini gauge on / off.
    const unHud = listen<boolean>("hud-toggled", (e) => {
      if (settings.hud.on !== e.payload) setHud({ on: e.payload });
    });
    // Every visit starts on the main view (not wherever it was left) with a fresh layout measurement.
    const unShown = listen("popup-shown", () => {
      view = "main";
      shownCount++;
      settleAfterShow();
      (document.activeElement as HTMLElement | null)?.blur?.();
      store.loadPing().catch(() => {});
      // The backend kept the charts going while the popup was closed.
      store.loadHistory().catch(() => {});
      setTimeout(refreshSizes, 300);
    });
    return () => {
      mq.removeEventListener("change", onMq);
      document.removeEventListener("visibilitychange", onShow);
      window.removeEventListener("focus", onShow);
      unlisten.then((f) => f());
      unShown.then((f) => f());
      unNotice.then((f) => f());
      stopUpdate();
      unOpenUpdate.then((f) => f());
      unHud.then((f) => f());
      unTheme.then((f) => f());
    };
  });

  function onKey(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    if (view === "update") leaveUpdate();
    else if (view !== "main") view = "main";
    else if (!settings.pinned) invoke("hide_popup");
  }

  async function setAutostart(on: boolean) {
    try {
      autostart = await invoke<boolean>("autostart_set", { enabled: on });
    } catch {
      autostart = await invoke<boolean>("autostart_get").catch(() => false);
    }
  }

  const tileKey: Record<TileId, Key> = {
    cpu: "cpu",
    memory: "memory",
    network: "network",
    disk: "disk",
    gpu: "gpu",
    battery: "battery",
    ping: "ping",
    processes: "processes",
    wifi: "wifi",
    system: "system",
  };
  function toggleTheme() {
    settings.theme = theme === "dark" ? "light" : "dark";
    settings.save();
  }
  const themeKey: Record<ThemeMode, Key> = { system: "themeSystem", dark: "themeDark", light: "themeLight" };
  /** System, Dark, Light: the same order in every app of the family (prefs.ts). */
  const themes: [ThemeMode, Key][] = THEME_MODES.map((id) => [id, themeKey[id]]);
  const styleKey: Record<StyleMode, Key> = { neumorphic: "styleNeumorphic", glass: "styleGlass" };
  const trayIconKey: Record<TrayIcon, Key> = { logo: "trayLogo", cpu: "sortCpu", gpu: "sortGpu", ram: "ram" };
  const historyKey: Record<number, Key> = { 180: "hist3m", 900: "hist15m", 3600: "hist1h" };
</script>

<svelte:window onkeydown={onKey} onresize={onResize} onmousemove={endResize} />

<!-- One tile, used by the main view and by the live preview in Settings > Modules > Customize. -->
{#snippet tileOf(id: TileId, mm: NonNullable<typeof m>, advMode: boolean, extra: number, onHistory: () => void)}
  {#if id === "cpu"}
    <CpuTile m={mm} adv={advMode} span={1} />
  {:else if id === "memory"}
    <MemoryTile m={mm} adv={advMode} span={1} />
  {:else if id === "network"}
    <NetworkTile m={mm} adv={advMode} span={1} />
  {:else if id === "disk"}
    <DiskTile m={mm} adv={advMode} span={1} />
  {:else if id === "gpu"}
    <GpuTile m={mm} adv={advMode} span={1} />
  {:else if id === "battery"}
    <BatteryTile m={mm} adv={advMode} span={1} {onHistory} />
  {:else if id === "ping"}
    <PingTile m={mm} adv={advMode} span={1} />
  {:else if id === "processes"}
    <ProcessesTile m={mm} adv={advMode} span={1} {extra} />
  {:else if id === "wifi"}
    <WifiTile m={mm} adv={advMode} span={1} />
  {:else if id === "system"}
    <SystemTile m={mm} adv={advMode} span={1} />
  {/if}
{/snippet}

<!-- Generous grab zones: the visible card is inset from the window edge by a transparent margin. -->
{#each edges as edge (edge.dir)}
  <div class="grip {edge.cls}" role="presentation" onmousedown={startResize(edge.dir)} ondblclick={fitContent}></div>
{/each}

<div class="shell" class:pinned={settings.pinned}>
  {#if settings.pinned}
    <!-- Square right corners + grip marks say "this edge can be dragged". -->
    <svg class="mark ne" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
      <path d="M2 1h9M5 4h6M8 7h3" />
    </svg>
    <svg class="mark se" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
      <path d="M2 11h9M5 8h6M8 5h3" />
    </svg>
  {/if}
  {#if view === "main"}
    <header class="top" data-tauri-drag-region>
      <div class="brand" data-tauri-drag-region>
        <span class="dot"></span>
        <h1>PulseTray</h1>
      </div>
      <div class="adv-toggle">
        <span>{tr("advancedShort")}</span>
        <button
          class="switch"
          class:on={settings.advanced}
          role="switch"
          aria-checked={settings.advanced}
          aria-label={tr("advancedMode")}
          use:tip={tr("advancedHint")}
          onclick={() => {
            settings.advanced = !settings.advanced;
            settings.save();
          }}><i></i></button
        >
      </div>
      {#if showFit}
        <button class="icon-btn" use:tip={tr("fitContent")} aria-label={tr("fitContent")} onclick={fitContent}>
          <Icon name="fit" />
        </button>
      {/if}
      <button
        class="icon-btn"
        class:pinned={settings.pinned}
        use:tip={tr("pin")}
        aria-label={tr("pin")}
        aria-pressed={settings.pinned}
        onclick={() => {
          settings.pinned = !settings.pinned;
          // Pinning keeps the current size: it is "fitted" if the user hasn't dragged it to a size of their own.
          if (settings.pinned) pinnedFitted = settings.sizes[adv ? "advanced" : "classic"] == null;
          settings.save();
        }}
      >
        <Icon name="pin" />
      </button>
      <!-- Quick light/dark switch: flips what is on screen now (the "System" choice stays in Settings). -->
      <button
        class="icon-btn"
        use:tip={theme === "dark" ? tr("themeToLight") : tr("themeToDark")}
        aria-label={theme === "dark" ? tr("themeToLight") : tr("themeToDark")}
        onclick={toggleTheme}
      >
        <Icon name={theme === "dark" ? "sun" : "moon"} />
      </button>
      <button class="icon-btn" title={tr("settings")} onclick={() => (view = "settings")}>
        <Icon name="gear" />
      </button>
    </header>

    {#if update.offer}
      <!-- Opens the update screen (version, notes, the Update button); nothing is installed by this click. -->
      <div class="notice upd">
        <button class="ntext" use:tip={tr("updateTip")} onclick={() => (view = "update")}>
          ↑&nbsp;&nbsp;{tr("menuUpdate").replace("{v}", update.offer.version)}
        </button>
      </div>
    {/if}
    {#if noticeShown && notice}
      <div class="notice" class:warn={notice.level === "warn"}>
        <button class="ntext" disabled={!notice.url} onclick={() => notice && notice.url && invoke("open_site", { url: notice.url })}>
          {notice.text}{#if notice.url}&nbsp;↗{/if}
        </button>
        <button class="nclose" aria-label={tr("dismissNotice")} use:tip={tr("dismissNotice")} onclick={dismissNotice}>✕</button>
      </div>
    {/if}

    <main bind:clientWidth={mainW}>
      {#if !m}
        <p class="empty">{tr("waiting")}</p>
      {:else if visibleTiles.length === 0}
        <p class="empty">{tr("noTiles")}</p>
      {:else}
        <div class="masonry" style="--cols:{cols}" bind:this={masonryEl}>
          {#each columns as col, ci (ci)}
            <div class="col">
              {#each col as id (id)}
                <div data-tile={id}>
                  {@render tileOf(id, m, adv, procExtra, () => (view = "battery"))}
                </div>
              {/each}
            </div>
          {/each}
        </div>
      {/if}
    </main>
  {:else if view === "update"}
    <header class="top">
      <button class="icon-btn" title={tr("back")} onclick={leaveUpdate}>
        <Icon name="back" />
      </button>
      <h1 class="grow">{tr("updateTitle")}</h1>
    </header>
    <main class="settings">
      <div bind:this={updEl}>
        <UpdateView current={version} onLater={leaveUpdate} />
      </div>
    </main>
  {:else if view === "battery"}
    <header class="top">
      <button class="icon-btn" title={tr("back")} onclick={() => (view = "main")}>
        <Icon name="back" />
      </button>
      <h1 class="grow">{tr("batHistory")}</h1>
    </header>
    <main class="settings">
      <BatteryHistory />
    </main>
  {:else}
    <header class="top">
      <button class="icon-btn" title={tr("back")} onclick={() => (view = "main")}>
        <Icon name="back" />
      </button>
      <h1 class="grow">{tr("settings")}</h1>
    </header>

    <main class="settings">
      <section class="card">
        <h3>{tr("appearance")}</h3>
        <div class="row">
          <span>{tr("theme")}</span>
          <div class="seg">
            {#each themes as [id, key] (id)}
              <button
                class:on={settings.theme === id}
                onclick={() => {
                  if (settings.theme === id) return;
                  settings.theme = id;
                  settings.save();
                }}>{tr(key)}</button
              >
            {/each}
          </div>
        </div>
        <div class="row style-row">
          <span use:tip={tr("styleTip")}>{tr("style")}</span>
          <div class="seg wrap" role="group" aria-label={tr("style")}>
            {#each STYLE_MODES as id (id)}
              <button
                class:on={settings.style === id}
                aria-pressed={settings.style === id}
                onclick={() => {
                  if (settings.style === id) return;
                  settings.style = id;
                  settings.save();
                }}>{tr(styleKey[id])}</button
              >
            {/each}
          </div>
        </div>
        <div class="row style-row">
          <span>{tr("language")}</span>
          <div class="seg wrap" role="group" aria-label={tr("language")}>
            {#each LANG_PREFS as id (id)}
              <button
                class:on={settings.langPref === id}
                aria-pressed={settings.langPref === id}
                onclick={() => {
                  if (settings.langPref === id) return;
                  settings.langPref = id;
                  settings.save();
                }}>{langChoiceLabel(id, tr("langAuto"), SYSTEM_LANG)}</button
              >
            {/each}
          </div>
        </div>
        <div class="row">
          <span use:tip={tr("trayIconTip")}>{tr("trayIcon")}</span>
          <div class="seg">
            {#each TRAY_ICONS as id (id)}
              <button
                class:on={settings.trayIcon === id}
                onclick={() => {
                  settings.trayIcon = id;
                  settings.save();
                }}>{tr(trayIconKey[id])}</button
              >
            {/each}
          </div>
        </div>
        <div class="row">
          <span use:tip={tr("chartHistoryTip")}>{tr("chartHistory")}</span>
          <div class="seg">
            {#each HISTORY_CHOICES as secs (secs)}
              <button
                class:on={settings.historySecs === secs}
                onclick={() => {
                  settings.historySecs = secs;
                  settings.save();
                }}>{tr(historyKey[secs])}</button
              >
            {/each}
          </div>
        </div>
      </section>

      <section class="card">
        <div class="row">
          <h3>{tr("modules")}</h3>
          <div class="seg" role="tablist" aria-label={tr("modules")}>
            {#each [["classic", "viewClassic"], ["advanced", "viewAdvanced"]] as [id, key] (id)}
              <button
                class:on={modTab === id}
                role="tab"
                aria-selected={modTab === id}
                onclick={() => {
                  modTab = id as ViewMode;
                  openDetail = null;
                }}>{tr(key as Key)}</button
              >
            {/each}
          </div>
        </div>
        <p class="sub hint">{tr("modulesHint")}</p>
        {#each settings.tileSets[modTab] as tile, i (tile.id)}
          {@const missingHw = absent(tile.id)}
          <div class="row mod">
            <button
              class="switch"
              class:on={tile.visible}
              role="switch"
              aria-checked={tile.visible}
              aria-label={tr(tileKey[tile.id])}
              onclick={() => settings.toggle(modTab, tile.id)}><i></i></button
            >
            <span class="grow">
              <Icon name={tile.id} size={14} />
              <span class="name">
                {tr(tileKey[tile.id])}
                {#if missingHw}<small>{tr(missingHw)}</small>{/if}
              </span>
            </span>
            {#if DETAIL[tile.id] && modTab === "advanced"}
              <button
                class="icon-btn sm"
                class:pinned={openDetail === tile.id}
                aria-expanded={openDetail === tile.id}
                aria-label={tr("detailOpen")}
                use:tip={tr("detailOpen")}
                onclick={() => (openDetail = openDetail === tile.id ? null : tile.id)}
              >
                <Icon name="sliders" size={14} />
              </button>
            {/if}
            <button class="icon-btn sm" disabled={i === 0} onclick={() => settings.move(modTab, i, -1)}>
              <Icon name="up" size={14} />
            </button>
            <button
              class="icon-btn sm"
              disabled={i === settings.tileSets[modTab].length - 1}
              onclick={() => settings.move(modTab, i, 1)}
            >
              <Icon name="down" size={14} />
            </button>
          </div>
          {#if openDetail === tile.id && DETAIL[tile.id] && modTab === "advanced"}
            {@const def = DETAIL[tile.id]}
            {@const lost = caps.gone(tile.id, tr)}
            <div class="detail">
              <!-- The tile as it looks with the current choices, live, so nobody has to go back to the main view. -->
              <div class="dprev">
                <span class="dl">{tr("detailPreview")}</span>
                <div class="dprev-card" inert aria-hidden="true">
                  {#if m}
                    {@render tileOf(tile.id, m, true, 0, () => {})}
                  {:else}
                    <p class="empty">{tr("waiting")}</p>
                  {/if}
                </div>
              </div>
              <div class="dopts">
              <p class="sub">{tr("detailHint")}</p>
              <span class="dl">{tr("detailRows")}</span>
              <div class="chips">
                {#each def.rows as [id, key] (id)}
                  {@const why = caps.why(tile.id, id)}
                  <button
                    class="chip"
                    class:on={!why && settings.shown(tile.id, id)}
                    class:off={why}
                    aria-pressed={!why && settings.shown(tile.id, id)}
                    aria-disabled={!!why}
                    use:tip={why ? tr(why) : ""}
                    onclick={() => !why && settings.toggleRow(tile.id, id)}>{tr(key)}</button
                  >
                {/each}
              </div>
              {#if def.badges}
                <span class="dl">{tr("detailBadge")}</span>
                <div class="chips" role="radiogroup" aria-label={tr("detailBadge")}>
                  {#each def.badges as [id, key] (id)}
                    {@const why = caps.why(tile.id, id)}
                    <button
                      class="chip"
                      class:on={caps.badge(tile.id) === id}
                      class:off={why}
                      role="radio"
                      aria-checked={caps.badge(tile.id) === id}
                      aria-disabled={!!why}
                      use:tip={why ? tr(why) : ""}
                      onclick={() => !why && settings.setBadge(tile.id, id)}>{tr(key)}</button
                    >
                  {/each}
                </div>
              {/if}
              {#if def.charts}
                <span class="dl">{tr("detailChart")}</span>
                <div class="chips" role="radiogroup" aria-label={tr("detailChart")}>
                  {#each def.charts as [id, key] (id)}
                    {@const why = caps.why(tile.id, id)}
                    <button
                      class="chip"
                      class:on={caps.chart(tile.id) === id}
                      class:off={why}
                      role="radio"
                      aria-checked={caps.chart(tile.id) === id}
                      aria-disabled={!!why}
                      use:tip={why ? tr(why) : ""}
                      onclick={() => !why && settings.setChart(tile.id, id)}>{tr(key)}</button
                    >
                  {/each}
                </div>
              {/if}
              {#each def.picks ?? [] as p (p.id)}
                <span class="dl">{tr(p.label)}</span>
                <div class="chips" role="radiogroup" aria-label={tr(p.label)}>
                  {#each p.options as [id, key] (id)}
                    <button
                      class="chip"
                      class:on={settings.pickOf(tile.id, p.id) === id}
                      role="radio"
                      aria-checked={settings.pickOf(tile.id, p.id) === id}
                      use:tip={key === "lenAuto" ? tr("lenAutoTip") : ""}
                      onclick={() => settings.setPick(tile.id, p.id, id)}>{tr(key)}</button
                    >
                  {/each}
                </div>
              {/each}
              {#if lost.length}
                <div class="capnote">
                  <span>{tr("capNote")}</span>
                  {#each lost as g (g.why)}
                    <span><b>{g.labels.join(", ")}:</b> {tr(g.why)}</span>
                  {/each}
                </div>
              {/if}
              <button class="small-btn wide" onclick={() => settings.resetDetail(tile.id)}>{tr("detailReset")}</button>
              </div>
            </div>
          {/if}
        {/each}
      </section>

      {#if pingOn}
        <section class="card">
          <h3>{tr("pingTargets")}</h3>
          <p class="sub hint">{tr("pingHint")}</p>
          {#each settings.pingTargets as t (t)}
            <div class="row mod">
              <span class="grow">{t === "gateway" ? tr("gateway") : t}</span>
              <button class="icon-btn sm" onclick={() => removeTarget(t)}><Icon name="x" size={14} /></button>
            </div>
          {/each}
          {#if settings.pingTargets.length < 6}
            <form
              class="row add"
              onsubmit={(e) => {
                e.preventDefault();
                addTarget();
              }}
            >
              <input bind:value={pingInput} placeholder={tr("hostPlaceholder")} maxlength="60" spellcheck="false" />
              <button class="small-btn" type="submit">{tr("addTarget")}</button>
            </form>
          {/if}
          <button class="small-btn wide" onclick={resetTargets}>{tr("resetTargets")}</button>
        </section>
      {/if}

      <section class="card">
        <h3>{tr("hudTitle")}</h3>
        <div class="row">
          <span class="grow" use:tip={tr("hudTip")}>{tr("hudOn")}</span>
          <button
            class="switch"
            class:on={settings.hud.on}
            role="switch"
            aria-checked={settings.hud.on}
            aria-label={tr("hudOn")}
            onclick={() => setHud({ on: !settings.hud.on })}><i></i></button
          >
        </div>
        {#if settings.hud.on}
          <div class="row">
            <span>{tr("hudItems")}</span>
            <div class="seg wrap">
              {#each HUD_ITEMS as id (id)}
                <button class:on={settings.hud.items.includes(id)} onclick={() => toggleHudItem(id)}>{hudLabel(id)}</button>
              {/each}
            </div>
          </div>
          {#if settings.hud.items.includes("fps")}
            <p class="sub hint">{tr("fpsHint")}</p>
            {#if settings.hud.hideFullscreen}<p class="sub hint">{tr("fpsHideFsHint")}</p>{/if}
            {#if m?.fps?.error}<p class="sub warnline">{tr(m.fps.error === "denied" ? "fpsDenied" : "fpsFailed")}</p>{/if}
          {/if}
          <div class="row">
            <span>{tr("hudOpacity")}</span>
            <div class="seg">
              {#each HUD_OPACITY as op (op)}
                <button class:on={Math.abs(settings.hud.opacity - op) < 0.01} onclick={() => setHud({ opacity: op })}
                  >{Math.round(op * 100)}%</button
                >
              {/each}
            </div>
          </div>
          <div class="row">
            <span class="grow" use:tip={tr("hudLockedTip")}>{tr("hudLocked")}</span>
            <button
              class="switch"
              class:on={settings.hud.locked}
              role="switch"
              aria-checked={settings.hud.locked}
              aria-label={tr("hudLocked")}
              onclick={() => setHud({ locked: !settings.hud.locked })}><i></i></button
            >
          </div>
          <div class="row">
            <span class="grow">{tr("hudHideFs")}</span>
            <button
              class="switch"
              class:on={settings.hud.hideFullscreen}
              role="switch"
              aria-checked={settings.hud.hideFullscreen}
              aria-label={tr("hudHideFs")}
              onclick={() => setHud({ hideFullscreen: !settings.hud.hideFullscreen })}><i></i></button
            >
          </div>
        {/if}
      </section>

      <section class="card">
        <h3>{tr("alertsTitle")}</h3>
        <p class="sub hint">{tr("alertsHint")}</p>
        {#each ALERT_ROWS as a (a.id)}
          {@const on = settings.alerts[a.id]}
          <div class="row">
            <span class="grow name2">
              {tr(a.label)}
              <small>{tr(a.sub)}</small>
            </span>
            <button
              class="switch"
              class:on
              role="switch"
              aria-checked={on}
              aria-label={tr(a.label)}
              onclick={() => settings.setAlerts({ [a.id]: !on } as Partial<AlertSettings>)}><i></i></button
            >
          </div>
          {#if on && a.choice}
            {@const c = a.choice}
            <div class="row choice">
              <span class="sub">{tr(c.label)}</span>
              <div class="seg">
                {#each c.options as v (v)}
                  <button
                    class:on={settings.alerts[c.key] === v}
                    onclick={() => settings.setAlerts({ [c.key]: v } as Partial<AlertSettings>)}>{c.text(v)}</button
                  >
                {/each}
              </div>
            </div>
          {/if}
        {/each}
        <div class="row">
          <button class="small-btn" onclick={testAlert}>{tr("alertTest")}</button>
        </div>
        {#if alertTestMsg}
          <p class="sub" class:warnline={alertTestMsg.bad}>{alertTestMsg.text}</p>
        {/if}
      </section>

      <section class="card">
        <h3>{tr("general")}</h3>
        <div class="row">
          <span class="grow" use:tip={tr("hotkeyTip")}>{tr("hotkey")} <kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>P</kbd></span>
          <button
            class="switch"
            class:on={settings.hotkey}
            role="switch"
            aria-checked={settings.hotkey}
            aria-label={tr("hotkey")}
            onclick={() => {
              settings.hotkey = !settings.hotkey;
              settings.save();
            }}><i></i></button
          >
        </div>
        {#if settings.hotkey && !hotkeyOk}<p class="sub warnline">{tr("hotkeyBusy")}</p>{/if}
        <div class="row">
          <span class="grow">{tr("autostart")}</span>
          <button
            class="switch"
            class:on={autostart}
            role="switch"
            aria-checked={autostart}
            aria-label={tr("autostart")}
            onclick={() => setAutostart(!autostart)}><i></i></button
          >
        </div>
        <div class="row">
          <span class="grow">{tr("autoCheck")}</span>
          <button
            class="switch"
            class:on={settings.autoCheck}
            role="switch"
            aria-checked={settings.autoCheck}
            aria-label={tr("autoCheck")}
            onclick={() => {
              settings.autoCheck = !settings.autoCheck;
              settings.save();
            }}><i></i></button
          >
        </div>
        <!-- Needs the check: greyed out (its own choice is kept) while "Check for Updates Automatically" is off. -->
        <div class="row">
          <span class="grow" class:dim={!settings.autoCheck}>{tr("autoUpdate")}</span>
          <button
            class="switch"
            class:on={settings.autoUpdate}
            role="switch"
            aria-checked={settings.autoUpdate}
            aria-label={tr("autoUpdate")}
            disabled={!settings.autoCheck}
            onclick={() => {
              settings.autoUpdate = !settings.autoUpdate;
              settings.save();
            }}><i></i></button
          >
        </div>
        <p class="sub hint">{tr("autoUpdateHint")}</p>
        <div class="row">
          <span class="sub">{tr("version")} {version}</span>
          <button class="small-btn check" disabled={checkBusy} onclick={checkNow}>
            {#if checkBusy}<span class="spin" aria-hidden="true"></span>{/if}
            {checkBusy ? tr("checking") : tr("checkNow")}
          </button>
        </div>
        {#if update.offer}
          <button class="upd-open" use:tip={tr("updateTip")} onclick={() => (view = "update")}>
            ↑&nbsp;&nbsp;{tr("menuUpdate").replace("{v}", update.offer.version)}
          </button>
        {:else if updateText}<p class="sub st {updateState}" class:fade={!checkBusy}>{updateText}</p>{/if}
        <button class="sitelink" onclick={() => invoke("open_site", { url: SITE })}>macik.com.tr</button>
        <button class="quit" onclick={() => invoke("quit_app")}>{tr("quit")}</button>
      </section>
    </main>
  {/if}
</div>

<Tooltip />

<style>
  .notice {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 18px 6px;
    padding: 6px 8px 6px 12px;
    border-radius: 12px;
    box-shadow: var(--inset-sm);
    font-size: 11.5px;
    line-height: 1.3;
    color: var(--text);
  }
  .notice.warn {
    color: var(--warn-text);
  }
  .ntext {
    flex: 1;
    min-width: 0;
    text-align: left;
    background: transparent;
    cursor: pointer;
  }
  .notice.upd {
    box-shadow: var(--raised-sm);
    padding: 3px 8px;
  }
  .notice.upd:active {
    box-shadow: var(--inset-sm);
  }
  .upd .ntext {
    font-weight: 700;
    color: var(--net-text);
    text-align: center;
    padding: 4px 0;
  }
  /* Settings: the result of "Check" when an update is on offer; opens the update screen. */
  .upd-open {
    align-self: flex-start;
    max-width: 100%;
    padding: 6px 12px;
    border-radius: 10px;
    box-shadow: var(--raised-sm);
    font-size: 12px;
    font-weight: 700;
    color: var(--net-text);
    text-align: left;
  }
  .upd-open:active {
    box-shadow: var(--inset-sm);
  }
  .small-btn.check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .spin {
    width: 11px;
    height: 11px;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-right-color: transparent;
    opacity: 0.8;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .st {
    font-weight: 600;
  }
  .st.ok {
    color: var(--ok-text);
  }
  .st.failed {
    color: var(--warn-text);
  }
  .st.fade {
    animation: fadein 0.35s ease-out;
  }
  @keyframes fadein {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }
  .ntext:disabled {
    cursor: default;
  }
  .nclose {
    flex: none;
    width: 22px;
    height: 22px;
    border-radius: 8px;
    background: transparent;
    color: var(--muted);
  }
  .nclose:hover {
    color: var(--text);
  }
  .sitelink {
    align-self: flex-start;
    background: transparent;
    color: var(--net-text);
    text-decoration: underline;
    font-size: 12px;
    padding: 2px 0;
  }
  .grip {
    position: fixed;
    z-index: 50;
  }
  .grip.n,
  .grip.s {
    left: 24px;
    right: 24px;
    height: 22px;
    cursor: ns-resize;
  }
  .grip.e,
  .grip.w {
    top: 24px;
    bottom: 24px;
    width: 22px;
    cursor: ew-resize;
  }
  .grip.n {
    top: 0;
  }
  .grip.s {
    bottom: 0;
  }
  .grip.e {
    right: 0;
  }
  .grip.w {
    left: 0;
  }
  .grip.ne,
  .grip.nw,
  .grip.se,
  .grip.sw {
    width: 30px;
    height: 30px;
  }
  .grip.ne {
    top: 0;
    right: 0;
    cursor: nesw-resize;
  }
  .grip.sw {
    bottom: 0;
    left: 0;
    cursor: nesw-resize;
  }
  .grip.nw {
    top: 0;
    left: 0;
    cursor: nwse-resize;
  }
  .grip.se {
    bottom: 0;
    right: 0;
    cursor: nwse-resize;
  }
  .shell {
    position: relative;
    margin: 14px;
    height: calc(100vh - 28px);
    background: var(--bg);
    border-radius: 28px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    container: shell / inline-size;
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--lo) 45%, transparent),
      0 8px 28px rgba(0, 0, 0, 0.28);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 16px 18px 8px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--net);
    box-shadow: 0 0 8px var(--net);
  }
  h1 {
    font-size: 15px;
    font-weight: 650;
  }
  .adv-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
    color: var(--muted);
    margin-left: auto;
  }
  /* A narrow popup (dragged small) gives up the header's text first, so every button, Settings included, stays reachable. */
  @container shell (max-width: 440px) {
    .brand h1 {
      display: none;
    }
  }
  @container shell (max-width: 330px) {
    .top {
      gap: 6px;
    }
    .adv-toggle span {
      display: none;
    }
  }
  @container shell (max-width: 260px) {
    .top {
      padding-left: 12px;
      padding-right: 12px;
    }
    .brand {
      display: none;
    }
    .top .icon-btn {
      width: 28px;
      height: 28px;
      border-radius: 10px;
    }
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  main {
    flex: 1;
    overflow-y: auto;
    /* room for the tile shadows so they aren't clipped by the scroll area */
    padding: 10px 18px 22px;
  }
  .masonry {
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    gap: 16px;
    align-items: start;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
  }
  .empty {
    text-align: center;
    color: var(--muted);
    margin-top: 60px;
  }

  /* settings */
  .settings {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
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
  .dim {
    opacity: 0.5;
  }
  .hint {
    margin-top: -6px;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .mod span.grow {
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }
  .seg {
    display: flex;
    padding: 3px;
    border-radius: 12px;
    box-shadow: var(--inset-sm);
    gap: 2px;
  }
  .seg.wrap {
    flex-wrap: wrap;
    justify-content: flex-end;
    min-width: 0;
  }
  /* Live preview on the left, the choices on the right; narrow windows stack them (preview first). */
  .detail {
    display: grid;
    grid-template-columns: minmax(300px, 1fr) minmax(0, 1fr);
    align-items: start;
    gap: 14px;
    margin: -2px 0 6px 0;
    padding: 10px 12px 12px;
    border-radius: 14px;
    box-shadow: var(--inset-sm);
  }
  .dopts,
  .dprev {
    display: flex;
    flex-direction: column;
    gap: 7px;
    min-width: 0;
  }
  /* stays in view while a long list of choices scrolls */
  .dprev {
    position: sticky;
    top: 6px;
  }
  .dprev-card {
    pointer-events: none;
  }
  @media (max-width: 600px) {
    .detail {
      grid-template-columns: minmax(0, 1fr);
    }
    .dprev {
      position: static;
    }
  }
  .dl {
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    margin-top: 2px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    padding: 4px 10px;
    border-radius: 9px;
    font-size: 11.5px;
    color: var(--muted);
    box-shadow: var(--raised-sm);
  }
  .chip.on {
    color: var(--net-text);
    font-weight: 600;
    box-shadow: var(--inset-sm);
  }
  /* can't be measured on this PC: flat and faded, the tooltip / the note below say why */
  .chip.off {
    opacity: 0.45;
    box-shadow: none;
    outline: 1px dashed color-mix(in srgb, var(--muted) 45%, transparent);
    outline-offset: -1px;
    cursor: default;
  }
  .capnote {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 11px;
    line-height: 1.4;
    color: var(--muted);
    margin-top: 2px;
  }
  .capnote b {
    font-weight: 600;
    color: var(--text);
  }
  .mod .name {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .mod .name small {
    font-size: 10.5px;
    color: var(--muted);
  }
  .name2 {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name2 small {
    font-size: 10.5px;
    line-height: 1.35;
    color: var(--muted);
  }
  .row.choice {
    margin: -4px 0 2px 12px;
  }
  /* the style choice has the longest labels: on a very narrow popup it drops under its label instead of splitting a label in two */
  .style-row {
    flex-wrap: wrap;
  }
  .style-row .seg {
    margin-left: auto;
  }
  .style-row .seg button {
    white-space: nowrap;
  }
  .seg button {
    padding: 5px 10px;
    border-radius: 9px;
    font-size: 12px;
    color: var(--muted);
  }
  .seg button.on {
    box-shadow: var(--raised-sm);
    color: var(--text);
    font-weight: 600;
  }
  kbd {
    display: inline-block;
    padding: 1px 6px;
    border-radius: 6px;
    box-shadow: var(--raised-sm);
    font: inherit;
    font-size: 10.5px;
    color: var(--muted);
  }
  .warnline {
    color: var(--warn-text);
    margin-top: -4px;
  }
  .shell.pinned {
    border-radius: 28px 5px 5px 28px;
  }
  .mark {
    position: absolute;
    right: 5px;
    z-index: 5;
    fill: none;
    stroke: var(--muted);
    stroke-width: 1.4;
    stroke-linecap: round;
    opacity: 0.8;
    pointer-events: none;
  }
  .mark.ne {
    top: 5px;
  }
  .mark.se {
    bottom: 5px;
  }
  .add {
    gap: 8px;
  }
  .add input {
    flex: 1;
    min-width: 0;
    padding: 7px 10px;
    border: 0;
    border-radius: 10px;
    background: var(--bg);
    box-shadow: var(--inset-sm);
    color: var(--text);
    font: inherit;
    font-size: 12px;
    user-select: text;
    -webkit-user-select: text;
    outline: none;
  }
  .quit {
    margin-top: 2px;
    padding: 10px;
    border-radius: 13px;
    box-shadow: var(--raised-sm);
    color: var(--bad-text);
    font-weight: 600;
  }
  .quit:active {
    box-shadow: var(--inset-sm);
  }
</style>
