import type { Lang } from "./i18n";
import { DETAIL, cleanDetails, type Detail } from "./tiledetail";
import { detectSystemLang, normalizeLangPref, normalizeTheme, resolveLang, type LangPref, type ThemeMode } from "./prefs";

export type { LangPref, ThemeMode };
/** The system language, looked up once at start (a change while the app runs is picked up at the next start). */
export const SYSTEM_LANG: Lang = detectSystemLang(typeof navigator !== "undefined" ? navigator.language : undefined);
/** The visual style: the soft Neumorphic look (the default) or frosted Glassmorphic panels. Independent of the theme. */
export type StyleMode = "neumorphic" | "glass";
export const STYLE_MODES: StyleMode[] = ["neumorphic", "glass"];
/**
 * Puts the style on the page (`data-style` on <html>, which app.css / glass.css key on). Neumorphic is the base CSS, so it
 * has no attribute at all and nothing of the glass rules applies. Every window (popup, tray menu, mini gauge) calls this.
 */
export function applyStyle(style: StyleMode) {
  if (style === "glass") document.documentElement.dataset.style = "glass";
  else delete document.documentElement.dataset.style;
}
/** What the tray icon shows: the logo, or the live CPU / GPU / RAM percentage. */
export type TrayIcon = "logo" | "cpu" | "gpu" | "ram";
// CPU, RAM, GPU: the same order as the tiles (and the mini gauge).
export const TRAY_ICONS: TrayIcon[] = ["logo", "cpu", "ram", "gpu"];
/** How far back the advanced charts reach, seconds (the backend keeps up to an hour). */
export const HISTORY_CHOICES = [180, 900, 3600] as const;
export type HistorySecs = (typeof HISTORY_CHOICES)[number];
export type TileId =
  | "cpu"
  | "memory"
  | "network"
  | "disk"
  | "gpu"
  | "battery"
  | "ping"
  | "processes"
  | "wifi"
  | "system";

export const ALL_TILES: TileId[] = [
  "cpu",
  "memory",
  "gpu",
  "network",
  "disk",
  "battery",
  "ping",
  "processes",
  "wifi",
  "system",
];

/** Tiles that start hidden: the user turns them on in Settings. */
const HIDDEN_BY_DEFAULT: TileId[] = ["wifi", "ping"];

export interface Size {
  w: number;
  h: number;
}

/** What the desktop mini gauge can show, in its order (FPS on top for games, then CPU, RAM, GPU — like the tiles). */
export const HUD_ITEMS = ["fps", "cpu", "ram", "gpu", "net", "disk"] as const;
export type HudItem = (typeof HUD_ITEMS)[number];

/** Desktop mini gauge (see hud.rs). Only the main window writes it; Rust gets it with `set_hud`. */
export interface HudSettings {
  on: boolean;
  items: HudItem[];
  /** Card opacity in steps of 0.1, 0.5 ... 1 (at the low end whatever moves behind the card shows through). */
  opacity: number;
  /** Clicks pass through to what is under the gauge (it can't be dragged then). */
  locked: boolean;
  hideFullscreen: boolean;
}

/** See hud.rs MIN_OPACITY. Settings offers 50 ... 100 % in steps of 10; an older stored value snaps to the nearest step. */
export const HUD_MIN_OPACITY = 0.5;

const defaultHud = (): HudSettings => ({
  on: false,
  items: ["cpu", "ram", "gpu", "net"],
  opacity: 0.9,
  locked: false,
  hideFullscreen: true,
});

function cleanHud(raw: unknown): HudSettings {
  const base = defaultHud();
  if (!raw || typeof raw !== "object") return base;
  const r = raw as Partial<HudSettings>;
  const items = Array.isArray(r.items) ? HUD_ITEMS.filter((k) => r.items!.includes(k)) : base.items;
  const op = Number(r.opacity);
  return {
    on: r.on === true,
    items,
    opacity: Number.isFinite(op) ? Math.min(1, Math.max(HUD_MIN_OPACITY, Math.round(op * 10) / 10)) : base.opacity,
    locked: r.locked === true,
    hideFullscreen: r.hideFullscreen !== false,
  };
}

/** "gateway" = the current default gateway; everything else is a host name or IPv4 address. */
export const DEFAULT_PING_TARGETS = ["gateway", "1.1.1.1", "8.8.8.8"];

interface Stored {
  theme: ThemeMode;
  style: StyleMode;
  /** "auto" (follow the system), "tr" or "en". */
  lang: LangPref;
  advanced: boolean;
  /** "Check for Updates Automatically": off = no daily background check (the manual "Check" always works). */
  autoCheck: boolean;
  /** "Install Updates Automatically" (only while `autoCheck` is on). */
  autoUpdate: boolean;
  dismissedNotice: string;
  pinned: boolean;
  hotkey: boolean;
  pingContinuous: boolean;
  pingTargets: string[];
  sizes: { classic: Size | null; advanced: Size | null };
  /** Which tiles are shown, in which order: one set for the classic view, one for the advanced view. */
  tileSets: TileSets;
  /** Per-tile choices for the advanced view (rows, badge, chart); tiles left at their defaults are absent. */
  details: Record<string, Detail>;
  /** The app list's tab when it was last switched (its "last used" opening tab). */
  procTab: string;
  historySecs: HistorySecs;
  trayIcon: TrayIcon;
  hud: HudSettings;
  alerts: AlertSettings;
}

/** Settings > Alerts: Windows notifications, every one off until the user turns it on. */
export interface AlertSettings {
  disk: boolean;
  diskPct: number;
  memory: boolean;
  temp: boolean;
  gpuTemp: number;
  health: boolean;
  hog: boolean;
  hogPct: number;
}
export const DISK_PCTS = [85, 90, 95];
export const GPU_TEMPS = [80, 85, 90];
export const HOG_PCTS = [30, 50, 80];

function defaultAlerts(): AlertSettings {
  return { disk: false, diskPct: 90, memory: false, temp: false, gpuTemp: 85, health: false, hog: false, hogPct: 50 };
}

function cleanAlerts(raw: unknown): AlertSettings {
  const base = defaultAlerts();
  if (!raw || typeof raw !== "object") return base;
  const r = raw as Record<string, unknown>;
  const on = (k: keyof AlertSettings) => r[k] === true;
  const pick = (v: unknown, opts: number[], d: number) => (typeof v === "number" && opts.includes(v) ? v : d);
  return {
    disk: on("disk"),
    diskPct: pick(r.diskPct, DISK_PCTS, base.diskPct),
    memory: on("memory"),
    temp: on("temp"),
    gpuTemp: pick(r.gpuTemp, GPU_TEMPS, base.gpuTemp),
    health: on("health"),
    hog: on("hog"),
    hogPct: pick(r.hogPct, HOG_PCTS, base.hogPct),
  };
}

export type TileEntry = { id: TileId; visible: boolean };
export type ViewMode = "classic" | "advanced";
export type TileSets = Record<ViewMode, TileEntry[]>;

/** Known ids only, each once (a duplicate would break the keyed lists in Settings), stored order kept, tiles added in
 *  newer versions appended; an older default order is replaced (visibility kept). */
function cleanTiles(raw: unknown, base: TileEntry[], orderVersion: number | undefined): TileEntry[] {
  const list = Array.isArray(raw) ? (raw as TileEntry[]) : [];
  const known = list
    .filter((x, i, all) => x && ALL_TILES.includes(x.id) && all.findIndex((y) => y && y.id === x.id) === i)
    .map((x) => ({ id: x.id, visible: x.visible !== false }));
  for (const t of base) {
    if (!known.some((x) => x.id === t.id)) known.push({ ...t });
  }
  if (orderVersion !== ORDER_VERSION) {
    known.sort((a, b) => ALL_TILES.indexOf(a.id) - ALL_TILES.indexOf(b.id));
    // v4: the System tile is off by default now.
    if ((orderVersion ?? 0) < 4) {
      const sys = known.find((x) => x.id === "system");
      if (sys) sys.visible = false;
    }
  }
  return known;
}

const KEY = "pulsetray.settings.v1";
/** Bump when the default tile order changes; older saved orders are reset (visibility is kept). */
const ORDER_VERSION = 4;

function defaults(): Stored {
  return {
    theme: "system",
    style: "neumorphic",
    lang: "auto",
    advanced: false,
    autoCheck: true,
    autoUpdate: true,
    dismissedNotice: "",
    pinned: false,
    hotkey: true,
    pingContinuous: false,
    pingTargets: [...DEFAULT_PING_TARGETS],
    sizes: { classic: null, advanced: null },
    tileSets: {
      classic: ALL_TILES.map((id) => ({ id, visible: !HIDDEN_BY_DEFAULT.includes(id) })),
      advanced: ALL_TILES.map((id) => ({ id, visible: !HIDDEN_BY_DEFAULT.includes(id) })),
    },
    details: {},
    procTab: "cpu",
    historySecs: 180,
    trayIcon: "logo",
    hud: defaultHud(),
    alerts: defaultAlerts(),
  };
}

function load(): Stored {
  const base = defaults();
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (!raw) return base;
    // Before Classic / Advanced had their own sets there was one list: both start from it.
    const sets = raw.tileSets && typeof raw.tileSets === "object" ? raw.tileSets : { classic: raw.tiles, advanced: raw.tiles };
    const tileSets: TileSets = {
      classic: cleanTiles(sets.classic, base.tileSets.classic, raw.orderVersion),
      advanced: cleanTiles(sets.advanced, base.tileSets.advanced, raw.orderVersion),
    };
    const size = (v: unknown): Size | null =>
      v && typeof v === "object" && Number((v as Size).w) > 0 && Number((v as Size).h) > 0
        ? { w: Number((v as Size).w), h: Number((v as Size).h) }
        : null;
    const targets =
      Array.isArray(raw.pingTargets) && raw.pingTargets.every((x: unknown) => typeof x === "string")
        ? [...new Set<string>(raw.pingTargets.map((x: string) => x.trim().toLowerCase()).filter(Boolean))].slice(0, 6)
        : [...DEFAULT_PING_TARGETS];
    return {
      theme: normalizeTheme(raw.theme, base.theme),
      // an unknown value (or a record from before the style existed) is the default look
      style: STYLE_MODES.includes(raw.style) ? raw.style : base.style,
      // a stored "tr" / "en" stays; a record without a language (or an unknown value) follows the system
      lang: normalizeLangPref(raw.lang, base.lang),
      advanced: raw.advanced === true,
      autoCheck: raw.autoCheck !== false,
      autoUpdate: raw.autoUpdate !== false,
      dismissedNotice: typeof raw.dismissedNotice === "string" ? raw.dismissedNotice.slice(0, 64) : "",
      pinned: raw.pinned === true,
      hotkey: raw.hotkey !== false,
      pingContinuous: raw.pingContinuous === true,
      pingTargets: targets,
      sizes: { classic: size(raw.sizes?.classic), advanced: size(raw.sizes?.advanced) },
      tileSets,
      details: cleanDetails(raw.details),
      procTab: ["cpu", "mem", "disk", "gpu"].includes(raw.procTab) ? raw.procTab : base.procTab,
      historySecs: HISTORY_CHOICES.includes(raw.historySecs) ? raw.historySecs : base.historySecs,
      trayIcon: TRAY_ICONS.includes(raw.trayIcon) ? raw.trayIcon : base.trayIcon,
      hud: cleanHud(raw.hud),
      alerts: cleanAlerts(raw.alerts),
    };
  } catch {
    return base;
  }
}

class Settings {
  theme = $state<ThemeMode>("system");
  style = $state<StyleMode>("neumorphic");
  /** The language CHOICE ("auto" / "tr" / "en"); `lang` below is what the UI is shown in. */
  langPref = $state<LangPref>("auto");
  advanced = $state(false);
  autoCheck = $state(true);
  autoUpdate = $state(true);
  dismissedNotice = $state("");
  pinned = $state(false);
  hotkey = $state(true);
  pingContinuous = $state(false);
  pingTargets = $state<string[]>([...DEFAULT_PING_TARGETS]);
  sizes = $state<{ classic: Size | null; advanced: Size | null }>({ classic: null, advanced: null });
  tileSets = $state<TileSets>({ classic: [], advanced: [] });
  details = $state<Record<string, Detail>>({});
  procTab = $state("cpu");
  historySecs = $state<HistorySecs>(180);
  trayIcon = $state<TrayIcon>("logo");
  hud = $state<HudSettings>(defaultHud());
  alerts = $state<AlertSettings>(defaultAlerts());

  /** The language in use: the choice, with "auto" resolved to the system language. */
  get lang(): Lang {
    return resolveLang(this.langPref, SYSTEM_LANG);
  }

  /** Nothing was stored yet: the very first start of this install (the main window acts on it once). */
  firstRun = false;

  constructor() {
    try {
      this.firstRun = localStorage.getItem(KEY) === null;
    } catch {
      this.firstRun = false; // storage unreadable: better not to act as a fresh install every time
    }
    const s = load();
    this.theme = s.theme;
    this.style = s.style;
    this.langPref = s.lang;
    this.advanced = s.advanced;
    this.autoCheck = s.autoCheck;
    this.autoUpdate = s.autoUpdate;
    this.dismissedNotice = s.dismissedNotice;
    this.pinned = s.pinned;
    this.hotkey = s.hotkey;
    this.pingContinuous = s.pingContinuous;
    this.pingTargets = s.pingTargets;
    this.sizes = s.sizes;
    this.tileSets = s.tileSets;
    this.details = s.details;
    this.procTab = s.procTab;
    this.historySecs = s.historySecs;
    this.trayIcon = s.trayIcon;
    this.hud = s.hud;
    this.alerts = s.alerts;
  }

  setAlerts(change: Partial<AlertSettings>) {
    this.alerts = { ...this.alerts, ...change };
    this.save();
  }

  /** Another window (the right-click menu, the mini gauge) re-reads what the main window saved. */
  refresh() {
    const s = load();
    this.theme = s.theme;
    this.style = s.style;
    this.langPref = s.lang;
    this.hud = s.hud;
  }

  save() {
    try {
      localStorage.setItem(
        KEY,
        JSON.stringify({
          theme: this.theme,
          style: this.style,
          lang: this.langPref,
          advanced: this.advanced,
          autoCheck: this.autoCheck,
          autoUpdate: this.autoUpdate,
          dismissedNotice: this.dismissedNotice,
          pinned: this.pinned,
          hotkey: this.hotkey,
          pingContinuous: this.pingContinuous,
          pingTargets: this.pingTargets,
          sizes: this.sizes,
          orderVersion: ORDER_VERSION,
          tileSets: this.tileSets,
          // what an older version (an update rolled back) reads
          tiles: this.tileSets.advanced,
          details: this.details,
          procTab: this.procTab,
          historySecs: this.historySecs,
          trayIcon: this.trayIcon,
          hud: this.hud,
          alerts: this.alerts,
        }),
      );
    } catch {
      /* storage unavailable: settings just won't persist */
    }
  }

  /** The tiles of the view in use (Classic and Advanced each have their own set). */
  get tiles(): TileEntry[] {
    return this.tileSets[this.advanced ? "advanced" : "classic"];
  }

  move(mode: ViewMode, index: number, delta: -1 | 1) {
    const list = this.tileSets[mode];
    const j = index + delta;
    if (j < 0 || j >= list.length) return;
    const next = [...list];
    [next[index], next[j]] = [next[j], next[index]];
    this.tileSets = { ...this.tileSets, [mode]: next };
    this.save();
  }

  // ---- per-tile customisation (advanced view)

  /** Is this row of the tile shown? */
  shown(tile: TileId, row: string): boolean {
    return !this.details[tile]?.hide.includes(row);
  }

  /** The chosen header badge / chart of a tile (the first option when left at the default). */
  badgeOf(tile: TileId): string {
    return this.details[tile]?.badge ?? DETAIL[tile]?.badges?.[0][0] ?? "none";
  }

  chartOf(tile: TileId): string {
    return this.details[tile]?.chart ?? DETAIL[tile]?.charts?.[0][0] ?? "none";
  }

  /** One of the tile's extra choices (see `DetailDef.picks`); the first option when left at the default. */
  pickOf(tile: TileId, pick: string): string {
    const def = DETAIL[tile]?.picks?.find((p) => p.id === pick);
    return this.details[tile]?.picks?.[pick] ?? def?.options[0][0] ?? "";
  }

  private editDetail(tile: TileId, change: (d: Detail) => void) {
    const cur = this.details[tile];
    const d: Detail = { hide: [...(cur?.hide ?? [])], badge: cur?.badge ?? null, chart: cur?.chart ?? null, picks: { ...cur?.picks } };
    change(d);
    if (!Object.keys(d.picks ?? {}).length) delete d.picks;
    const next = { ...this.details };
    if (d.hide.length || d.badge || d.chart || d.picks) next[tile] = d;
    else delete next[tile];
    this.details = next;
    this.save();
  }

  toggleRow(tile: TileId, row: string) {
    this.editDetail(tile, (d) => {
      d.hide = d.hide.includes(row) ? d.hide.filter((r) => r !== row) : [...d.hide, row];
    });
  }

  setBadge(tile: TileId, badge: string) {
    this.editDetail(tile, (d) => (d.badge = badge === DETAIL[tile]?.badges?.[0][0] ? null : badge));
  }

  setChart(tile: TileId, chart: string) {
    this.editDetail(tile, (d) => (d.chart = chart === DETAIL[tile]?.charts?.[0][0] ? null : chart));
  }

  setPick(tile: TileId, pick: string, value: string) {
    const def = DETAIL[tile]?.picks?.find((p) => p.id === pick);
    this.editDetail(tile, (d) => {
      const picks = { ...d.picks };
      if (value === def?.options[0][0]) delete picks[pick];
      else picks[pick] = value;
      d.picks = picks;
    });
  }

  resetDetail(tile: TileId) {
    this.editDetail(tile, (d) => {
      d.hide = [];
      d.badge = null;
      d.chart = null;
      d.picks = {};
    });
  }

  toggle(mode: ViewMode, id: TileId) {
    const tile = this.tileSets[mode].find((x) => x.id === id);
    if (tile) tile.visible = !tile.visible;
    this.save();
  }
}

export const settings = new Settings();
