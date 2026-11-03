import type { Key } from "./i18n";

/**
 * What the user can tailor per tile in the advanced view (Settings > Modules > Customize): which rows are shown,
 * what the header badge says and what the chart draws. The first badge / chart option is the default.
 */
export interface DetailDef {
  rows: [string, Key][];
  badges?: [string, Key][];
  charts?: [string, Key][];
  /** More one-of choices (the first option is the default), e.g. the app list's opening tab. */
  picks?: PickDef[];
}

export interface PickDef {
  id: string;
  label: Key;
  options: [string, Key][];
}

export const DETAIL: Record<string, DetailDef> = {
  cpu: {
    rows: [
      ["model", "model"],
      ["cores", "coresLabel"],
      ["clock", "clock"],
      ["cache", "cache"],
      ["temp", "temperature"],
      ["power", "cpuPower"],
      ["perCore", "perCore"],
    ],
    badges: [
      ["temp", "temperature"],
      ["clock", "clock"],
      ["usage", "usage"],
      ["none", "none"],
    ],
    charts: [
      ["usage", "usage"],
      ["temp", "temperature"],
      ["power", "cpuPower"],
      ["clock", "clock"],
      ["none", "none"],
    ],
  },
  memory: {
    rows: [
      ["swap", "swap"],
      ["used", "used"],
      ["available", "available"],
      ["cached", "cachedMem"],
      ["commit", "commit"],
      ["type", "memType"],
      ["modules", "memModules"],
      ["maker", "maker"],
    ],
    badges: [
      ["combined", "memCombined"],
      ["commit", "commit"],
      ["ram", "ram"],
      ["swap", "swap"],
      ["none", "none"],
    ],
    charts: [
      ["ram", "ram"],
      ["swap", "swap"],
      ["none", "none"],
    ],
  },
  gpu: {
    rows: [
      ["model", "model"],
      ["usage", "usage"],
      ["temp", "temperature"],
      ["power", "power"],
      ["coreClock", "coreClock"],
      ["memClock", "memClock"],
      ["fan", "fan"],
      ["vram", "vram"],
      ["shared", "sharedMem"],
    ],
    badges: [
      ["temp", "temperature"],
      ["usage", "usage"],
      ["power", "power"],
      ["vram", "vram"],
      ["none", "none"],
    ],
    charts: [
      ["usage", "usage"],
      ["temp", "temperature"],
      ["vram", "vram"],
      ["power", "power"],
      ["none", "none"],
    ],
  },
  network: {
    rows: [
      ["downChart", "downChart"],
      ["upChart", "upChart"],
      ["adapter", "adapter"],
      ["ip", "ip"],
      ["linkSpeed", "linkSpeed"],
      ["totals", "netTotals"],
    ],
    badges: [
      ["link", "linkSpeed"],
      ["none", "none"],
    ],
  },
  disk: {
    rows: [
      ["drives", "drives"],
      ["active", "active"],
      ["read", "read"],
      ["write", "write"],
      ["models", "diskModels"],
      ["health", "diskHealth"],
    ],
    badges: [
      ["active", "active"],
      ["temp", "driveTemp"],
      ["none", "none"],
    ],
    charts: [
      ["none", "none"],
      ["active", "active"],
      ["read", "read"],
      ["write", "write"],
    ],
  },
  battery: {
    rows: [
      ["health", "health"],
      ["capacity", "energyLeft"],
      ["power", "power"],
      ["time", "timeLeft"],
      ["voltage", "voltage"],
      ["cycles", "cycles"],
      ["powerChart", "powerChart"],
      ["chargeChart", "chargeChart"],
    ],
    badges: [
      ["state", "power"],
      ["none", "none"],
    ],
  },
  wifi: {
    rows: [
      ["ssid", "ssid"],
      ["signal", "signal"],
      ["standard", "standard"],
      ["rate", "rate"],
    ],
    badges: [
      ["signal", "signal"],
      ["none", "none"],
    ],
  },
  ping: {
    rows: [
      ["chart", "detailChart"],
      ["stats", "pingStats"],
      ["ip", "ip"],
    ],
    badges: [
      ["avg", "pingAvg"],
      ["latest", "latest"],
      ["jitter", "jitter"],
      ["loss", "loss"],
      ["none", "none"],
    ],
  },
  processes: {
    rows: [
      ["tabs", "procTabs"],
      ["icons", "procIcons"],
      ["multi", "procMulti"],
    ],
    badges: [
      ["procs", "procCount"],
      ["threads", "threadCount"],
      ["none", "none"],
    ],
    picks: [
      {
        id: "tab",
        label: "pickTab",
        options: [
          ["cpu", "sortCpu"],
          ["mem", "sortMem"],
          ["disk", "sortDisk"],
          ["gpu", "sortGpu"],
          ["last", "lastUsed"],
        ],
      },
      {
        id: "length",
        label: "pickLength",
        options: [
          ["auto", "lenAuto"],
          ["3", "len3"],
          ["5", "len5"],
          ["10", "len10"],
        ],
      },
    ],
  },
  system: {
    rows: [
      ["os", "os"],
      ["build", "buildNo"],
      ["board", "board"],
      ["uptime", "uptimeFull"],
      ["procs", "procCount"],
      ["threads", "threadCount"],
      ["handles", "handles"],
    ],
    badges: [
      ["uptime", "uptimeFull"],
      ["none", "none"],
    ],
  },
};

/** One tile's choices; `null` = the default (first option). */
export interface Detail {
  hide: string[];
  badge: string | null;
  chart: string | null;
  /** Pick id -> chosen option; only choices other than the default are stored. */
  picks?: Record<string, string>;
}

/** Keeps only what the definitions know (an old or hand-edited setting can't break a tile). */
export function cleanDetails(raw: unknown): Record<string, Detail> {
  const out: Record<string, Detail> = {};
  if (!raw || typeof raw !== "object") return out;
  for (const [id, def] of Object.entries(DETAIL)) {
    const d = (raw as Record<string, unknown>)[id] as Partial<Detail> | undefined;
    if (!d || typeof d !== "object") continue;
    const rows = def.rows.map(([r]) => r);
    const hide = Array.isArray(d.hide) ? d.hide.filter((r): r is string => typeof r === "string" && rows.includes(r)) : [];
    const pick = (v: unknown, opts?: [string, Key][]) =>
      typeof v === "string" && opts?.some(([o]) => o === v) ? v : null;
    const picks: Record<string, string> = {};
    for (const p of def.picks ?? []) {
      const v = (d.picks as Record<string, unknown> | undefined)?.[p.id];
      if (typeof v === "string" && v !== p.options[0][0] && p.options.some(([o]) => o === v)) picks[p.id] = v;
    }
    const entry: Detail = { hide: [...new Set(hide)], badge: pick(d.badge, def.badges), chart: pick(d.chart, def.charts) };
    if (Object.keys(picks).length) entry.picks = picks;
    if (entry.hide.length || entry.badge || entry.chart || entry.picks) out[id] = entry;
  }
  return out;
}
