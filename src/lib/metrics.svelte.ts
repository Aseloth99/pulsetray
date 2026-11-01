import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { settings } from "./settings.svelte";

export interface DiskInfo {
  name: string;
  mount: string;
  fs: string;
  kind: string;
  total: number;
  used: number;
}

export interface BatteryDetail {
  designMwh: number;
  fullMwh: number;
  currentMwh: number;
  cycles: number;
  /** Positive while charging, negative while discharging (mW). */
  rateMw: number;
  voltageMv: number;
}

export interface Battery {
  present: boolean;
  percent: number;
  charging: boolean;
  plugged: boolean;
  remainingSecs: number | null;
  detail: BatteryDetail | null;
}

export interface GpuSensors {
  tempC: number | null;
  coreMhz: number | null;
  memMhz: number | null;
  fanRpm: number | null;
  fanPct: number | null;
  powerW: number | null;
  powerLimitW: number | null;
}

export interface Gpu {
  luid: string;
  name: string;
  usage: number;
  vramUsed: number;
  vramTotal: number;
  sharedUsed: number;
  sharedTotal: number;
  sensors: GpuSensors;
}

export interface Proc {
  name: string;
  cpu: number;
  mem: number;
  /** Disk I/O in bytes per second. */
  disk: number;
  gpu: number;
  count: number;
  /** Every process of this name (the list groups them). */
  pids: number[];
}

export interface Wifi {
  connected: boolean;
  /** The adapter's name. */
  adapter: string;
  /** Not connected because the radio is switched off. */
  radioOff: boolean;
  ssid: string;
  signal: number;
  rxMbps: number;
  txMbps: number;
  standard: string;
  /** Not connected: networks in reach and the strongest one. */
  networks: number;
  bestSsid: string;
  bestSignal: number;
}

export interface PingSample {
  target: string;
  ip: string | null;
  rtt: number | null;
}

export interface PingStatus {
  running: boolean;
  unlimited: boolean;
  remainingSecs: number | null;
  continuous: boolean;
  paused: boolean;
  lastTestAt: number | null;
  lastTestPlannedSecs: number | null;
  lastStopReason: "finished" | "stopped" | "fullscreen" | null;
}

interface PingEvent {
  status: PingStatus;
  samples: PingSample[];
}

interface HistoryDto {
  /** true: the whole window (replace); false: only the samples after the `since` that was asked for. */
  full: boolean;
  times: number[];
  series: Record<string, Series>;
  chargeTimes: number[];
  charge: number[];
}

interface PingSnapshot {
  status: PingStatus;
  targets: { target: string; ip: string | null; hist: (number | null)[] }[];
}

export interface NvmeHealth {
  criticalWarning: number;
  sparePct: number;
  spareThresholdPct: number;
  usedPct: number;
  readBytes: number;
  writtenBytes: number;
  powerCycles: number;
  powerOnHours: number;
  unsafeShutdowns: number;
  mediaErrors: number;
}

export interface DiskHealth {
  index: number;
  tempC: number | null;
  warnC: number | null;
  critC: number | null;
  nvme: NvmeHealth | null;
  /** "good" | "warn" | "bad" | "" (only the temperature is known) */
  status: string;
}

export interface FpsReading {
  /** null while the app in front draws nothing (desktop, a document, a paused app). */
  fps: number | null;
  frameMs: number | null;
  /** The app in front (exe name). */
  app: string;
  /** "denied": Windows refused the trace session (not admin / not in Performance Log Users); "failed". */
  error: string;
}

export interface Metrics {
  /** When the backend took the sample (ms since the epoch). */
  tsMs?: number;
  cpuName: string;
  cpuCores: number;
  cpuPhysical: number;
  cpuUsage: number;
  cpuPerCore: number[];
  /** Clock of every logical processor (MHz), same order as cpuPerCore; may be empty. */
  cpuCoreMhz: number[];
  cpuFreqMhz: number;
  cpuTempC: number | null;
  cpuPowerW: number | null;
  /** Where the value comes from: "acpi" / "meter" (Windows) or "" when there is none. */
  cpuTempSrc?: string;
  cpuPowerSrc?: string;
  memTotal: number;
  memUsed: number;
  memAvailable: number;
  memCached: number;
  commitUsed: number;
  commitLimit: number;
  swapTotal: number;
  swapUsed: number;
  netDown: number;
  netUp: number;
  netBootDown: number;
  netBootUp: number;
  netSessionDown: number;
  netSessionUp: number;
  netTodayDown: number;
  netTodayUp: number;
  netAdapter: { name: string; ip: string; mac: string } | null;
  diskRead: number;
  diskWrite: number;
  diskActive: number;
  disks: DiskInfo[];
  /** Temperature / wear per physical drive (diskhealth.rs); empty when no drive reports it. */
  diskHealth?: DiskHealth[];
  gpus: Gpu[];
  battery: Battery;
  wifi: Wifi | null;
  /** Frames per second of the app in front (fps.rs); only while the mini gauge shows FPS. */
  fps?: FpsReading | null;
  topCpu: Proc[];
  topMem: Proc[];
  topDisk: Proc[];
  topGpu: Proc[];
  processCount: number;
  threadCount: number;
  handleCount: number;
  uptimeSecs: number;
}

export interface Hardware {
  cpuName: string;
  cpuMaxMhz: number;
  cpuCores: number;
  cpuLogical: number;
  cpuL2Kb: number;
  cpuL3Kb: number;
  memModules: {
    capacity: number;
    ratedMts: number;
    configuredMts: number;
    kind: string;
    maker: string;
    slot: string;
  }[];
  memSlots: number;
  board: string;
  os: string;
  osBuild: string;
  gpus: { luid: string; name: string; vramTotal: number; sharedTotal: number }[];
  drives: { model: string; size: number; media: string; bus: string; index?: number | null }[];
  adapters: { name: string; description: string; linkMbps: number; kind: string }[];
}

/**
 * How far back the charts reach is the user's choice (Settings: 3 min, 15 min, 1 h). The backend keeps recording
 * while the popup is closed (every 2 s) and hands over what the UI missed when the popup opens. Samples are at
 * least a second apart, so a window of N seconds never holds more than N samples.
 */
const GIB = 1024 ** 3;

export type Series = (number | null)[];

function push<T>(arr: T[] | undefined, v: T, cap: number): T[] {
  const a = arr ?? [];
  const next = a.length >= cap ? a.slice(a.length - cap + 1) : a.slice();
  next.push(v);
  return next;
}

const share = (used: number, total: number) => (total > 0 ? Math.min(100, (used / total) * 100) : null);

/** Memory a GPU is really working from: dedicated VRAM, or shared system memory on an integrated GPU. */
export function gpuMemShare(g: Gpu): number | null {
  return g.vramTotal >= GIB ? share(g.vramUsed, g.vramTotal) : share(g.sharedUsed, g.sharedTotal);
}

const PING_HISTORY = 300;


/** The GPU worth showing as "the" GPU: the one with the most dedicated memory (dGPU over iGPU). */
export function primaryGpu(gpus: Gpu[]): Gpu | undefined {
  return gpus.reduce<Gpu | undefined>(
    (best, g) => (!best || g.vramTotal > best.vramTotal ? g : best),
    undefined,
  );
}

class Store {
  data = $state<Metrics | null>(null);
  hw = $state<Hardware | null>(null);
  /** When each chart sample arrived (ms). Every series is aligned to the END of this list. */
  times = $state.raw<number[]>([]);
  /**
   * Chart history per metric: "cpu.usage", "mem.ram", "net.down", "gpu.<luid>.temp", ... (null = not measured
   * at that moment). A GPU that goes away takes its series with it, so a series never has holes in the middle.
   */
  series = $state.raw<Record<string, Series>>({});
  /** Battery charge %, 1 sample per 5 s (last 10 minutes), with its own timestamps. */
  chargeHist = $state.raw<number[]>([]);
  chargeTimes = $state.raw<number[]>([]);
  /** Latency history per target (null = timeout); the backend keeps up to 300 samples (5 min). */
  ping = $state<Record<string, { ip: string | null; hist: (number | null)[] }>>({});
  pingStatus = $state<PingStatus | null>(null);
  /** A history request is on its way: live samples only update the numbers until it is merged. */
  private loading = 0;

  private apply(m: Metrics) {
    this.data = m;
    // The backend has this sample too; the history about to be merged would otherwise end up out of order.
    if (this.loading) return;
    const cap = settings.historySecs;
    const now = m.tsMs ?? Date.now();
    const last = this.times.length ? this.times[this.times.length - 1] : 0;
    if (now <= last) {
      // Already in the history fetched from the backend (the popup just opened): only the numbers above change.
      if (last - now < 5000) return;
      // The clock was set back (time sync): start the charts over instead of freezing until it catches up.
      this.times = [];
      this.series = {};
      this.chargeTimes = [];
      this.chargeHist = [];
    }
    this.times = push(this.times, now, cap);
    const old = this.series;
    const next: Record<string, Series> = {};
    const put = (key: string, v: number | null | undefined) => (next[key] = push(old[key], v ?? null, cap));
    put("cpu.usage", m.cpuUsage);
    put("cpu.temp", m.cpuTempC);
    put("cpu.power", m.cpuPowerW);
    put("cpu.clock", m.cpuFreqMhz || null);
    put("mem.ram", share(m.memUsed, m.memTotal) ?? 0);
    put("mem.swap", share(m.swapUsed, m.swapTotal));
    put("net.down", m.netDown);
    put("net.up", m.netUp);
    put("disk.active", m.diskActive);
    put("disk.read", m.diskRead);
    put("disk.write", m.diskWrite);
    for (const g of m.gpus) {
      put(`gpu.${g.luid}.usage`, g.usage);
      put(`gpu.${g.luid}.temp`, g.sensors.tempC);
      put(`gpu.${g.luid}.vram`, gpuMemShare(g));
      put(`gpu.${g.luid}.power`, g.sensors.powerW);
    }
    const d = m.battery.present ? m.battery.detail : null;
    put("bat.power", d ? Math.abs(d.rateMw) / 1000 : null);
    this.series = next;
    const lastCharge = this.chargeTimes.length ? this.chargeTimes[this.chargeTimes.length - 1] : 0;
    if (m.battery.present && now - lastCharge >= 5000) {
      this.chargeHist = push(this.chargeHist, m.battery.percent, 120);
      this.chargeTimes = push(this.chargeTimes, now, 120);
    }
  }

  private applyPing(e: PingEvent) {
    // A new test starts from empty graphs (the backend clears its copy at the same moment).
    const fresh = e.status.running && !this.pingStatus?.running;
    this.pingStatus = e.status;
    if (e.samples.length === 0 && !fresh) return;
    const next: typeof this.ping = fresh ? {} : { ...this.ping };
    for (const s of e.samples) {
      next[s.target] = { ip: s.ip, hist: push(next[s.target]?.hist, s.rtt, PING_HISTORY) };
    }
    this.ping = next;
  }

  /**
   * The charts' history as the backend recorded it, including the time the popup was closed: only what is
   * missing since the newest sample here, or (`full`, or when the backend says so) the whole chart window.
   */
  async loadHistory(full = false) {
    const since = !full && this.times.length ? this.times[this.times.length - 1] : null;
    const secs = settings.historySecs;
    this.loading++;
    try {
      const h = await invoke<HistoryDto | null>("get_history", { since, secs });
      if (!h) return;
      if (h.full) {
        if (!h.times.length) return;
        this.times = h.times;
        this.series = h.series;
      } else if (h.times.length) {
        const merged = appendHistory(this.times, this.series, h.times, h.series, secs);
        this.times = merged.times;
        this.series = merged.series;
      }
      this.chargeTimes = h.chargeTimes;
      this.chargeHist = h.charge;
    } finally {
      this.loading--;
    }
  }

  /** Pull the authoritative copy (graphs + status) from the backend, e.g. when the popup is shown. */
  async loadPing() {
    const snap = await invoke<PingSnapshot>("get_ping_state");
    this.pingStatus = snap.status;
    const next: typeof this.ping = {};
    for (const t of snap.targets) next[t.target] = { ip: t.ip, hist: t.hist };
    this.ping = next;
  }

  async start() {
    await listen<Metrics>("metrics", (e) => this.apply(e.payload));
    await listen<Hardware>("hardware", (e) => (this.hw = e.payload));
    await listen<PingEvent>("ping", (e) => this.applyPing(e.payload));
    await this.loadHistory().catch(() => {});
    // Show something immediately when the popup opens, before the next tick arrives.
    const first = await invoke<Metrics>("get_metrics");
    if (first.cpuCores > 0) this.apply(first);
    // The "hardware" event may already have delivered it; a null from the (earlier) read must not erase that.
    this.hw = (await invoke<Hardware | null>("get_hardware")) ?? this.hw;
    await this.loadPing();
  }
}

export const store = new Store();

/**
 * `times`/`series` followed by newer samples, cut to the last `cap` samples. A series can be shorter than its
 * times (a GPU that appeared later): it is aligned to the end, and the gap is filled with nulls when merging.
 */
export function appendHistory(
  times: number[],
  series: Record<string, Series>,
  moreTimes: number[],
  more: Record<string, Series>,
  cap: number,
): { times: number[]; series: Record<string, Series> } {
  const n0 = times.length;
  const n1 = moreTimes.length;
  const cut = Math.max(0, n0 + n1 - cap);
  const out: Record<string, Series> = {};
  for (const key of new Set([...Object.keys(series), ...Object.keys(more)])) {
    const a = series[key] ?? [];
    const b = more[key] ?? [];
    const joined: Series = [
      ...new Array<null>(n0 - a.length).fill(null),
      ...a,
      ...new Array<null>(n1 - b.length).fill(null),
      ...b,
    ];
    out[key] = joined.slice(cut);
  }
  return { times: [...times, ...moreTimes].slice(cut), series: out };
}
