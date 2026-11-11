import type { Key } from "./i18n";
import type { Metrics } from "./metrics.svelte";
import { store } from "./metrics.svelte";
import { settings, type TileId } from "./settings.svelte";
import { DETAIL, addedChart, movedChart, removedChart, replacedChart, type QuickPick } from "./tiledetail";

/** Extra, per-view availability of a chart (e.g. the GPU shown has no power sensor); the global `why` still applies. */
export type ChartHas = (id: string) => boolean;
import { tr } from "./tr";

/** Always-measured values: missing for this long (not one failed read) before an option is greyed out. */
const HOLD_MS = 5000;
/** GPU sensors are read only while the GPU tile is on screen and the card is awake: judged after this many such reads. */
const GPU_READS = 5;
const GPU_FIELDS: [string, (s: Metrics["gpus"][number]["sensors"]) => unknown][] = [
  ["temp", (s) => s.tempC],
  ["power", (s) => s.powerW],
  ["coreClock", (s) => s.coreMhz],
  ["memClock", (s) => s.memMhz],
  ["fan", (s) => s.fanPct ?? s.fanRpm],
];

/**
 * Which tile options this PC can actually show. Settings > Modules > Customize greys out the rest and says why, and
 * a tile whose chosen badge / chart can't be measured shows the first option that can instead of an empty one (the
 * choice itself is kept: when the value turns up later, it is shown again).
 * Nothing is greyed out on a guess: an option counts as available until its value is known to be missing.
 */
class Caps {
  /** "tile.option" -> why it can't be shown (i18n key). */
  missing = $state<Record<string, Key>>({});
  private firstSample = 0;
  private lastSeen: Record<string, number> = {};
  private gpuReads = 0;
  private gpuSeen = new Set<string>();

  /** `gpuMeasured`: the GPU tile is on screen, so the backend reads the card's sensors. */
  observe(m: Metrics, gpuMeasured: boolean) {
    const now = performance.now();
    if (!this.firstSample) this.firstSample = now;
    const next: Record<string, Key> = {};
    const live = (cap: string, has: boolean, why: Key) => {
      if (has) this.lastSeen[cap] = now;
      else if (now - (this.lastSeen[cap] ?? this.firstSample) >= HOLD_MS) next[cap] = why;
    };
    live("cpu.temp", m.cpuTempC != null, "capCpuTemp");
    live("cpu.power", m.cpuPowerW != null, "capCpuPower");
    live("memory.swap", m.swapTotal > 0, "capNoPagefile");
    // The per-disk counters: tabs and the per-disk list need them (the totals come from elsewhere).
    live("disk.perDisk", (m.physDisks?.length ?? 0) > 0, "capNotReported");
    live("disk.tabs", (m.physDisks?.length ?? 0) > 0, "capNotReported");
    if (m.diskHealth) {
      live("disk.temp", m.diskHealth.some((h) => h.tempC != null), "capDriveTemp");
      live("disk.health", m.diskHealth.length > 0, "capDriveHealth");
    }

    // Sensors are probed only on a card in use (a sleeping laptop GPU must not be woken), and a value any card has
    // reported once stays available: the other card of an iGPU + dGPU pair may simply not have it.
    if (gpuMeasured) {
      const probed = m.gpus.filter((g) => g.usage > 0 || g.vramUsed > 0);
      if (probed.length) this.gpuReads++;
      for (const g of probed) for (const [id, get] of GPU_FIELDS) if (get(g.sensors) != null) this.gpuSeen.add(id);
    }
    if (this.gpuReads >= GPU_READS) {
      for (const [id] of GPU_FIELDS) if (!this.gpuSeen.has(id)) next[`gpu.${id}`] = "capGpuSensor";
    }

    const hw = store.hw;
    if (hw) {
      if (!hw.cpuL2Kb && !hw.cpuL3Kb) next["cpu.cache"] = "capNotReported";
      if (!hw.memModules.length) {
        next["memory.type"] = "capNotReported";
        next["memory.modules"] = "capNotReported";
      }
      if (!hw.memModules.some((x) => x.maker && x.maker !== "Unknown")) next["memory.maker"] = "capNotReported";
      if (!hw.board) next["system.board"] = "capNotReported";
    }

    const a = Object.entries(next).sort().join();
    if (a !== Object.entries(this.missing).sort().join()) this.missing = next;
  }

  /** Why this option can't be shown on this PC, or undefined when it can. */
  why(tile: TileId, option: string): Key | undefined {
    return this.missing[`${tile}.${option}`];
  }

  /** The header badge / chart a tile shows: the chosen one, or the first option this PC can show. */
  badge(tile: TileId): string {
    return this.pick(tile, settings.badgeOf(tile), DETAIL[tile]?.badges);
  }

  /**
   * The charts a tile shows, in order: the chosen ones this PC can measure. When none of the chosen ones can be
   * measured the first option that can stands in (a chart that was asked for is never replaced by an empty one);
   * the choice itself is kept.
   */
  charts(tile: TileId, has?: ChartHas): string[] {
    const chosen = settings.chartsOf(tile);
    const usable = (id: string) => !this.why(tile, id) && (has?.(id) ?? true);
    const ok = chosen.filter(usable);
    if (ok.length || !chosen.length) return ok;
    const first = DETAIL[tile]?.charts?.find(([o]) => o !== "none" && usable(o))?.[0];
    return first ? [first] : [];
  }

  /** Changes the tile's chart list. A stand-in (see `charts`) isn't part of the stored choice: it starts from what is shown. */
  editCharts(tile: TileId, change: (ids: string[]) => string[], has?: ChartHas) {
    const stored = settings.chartsOf(tile);
    const shown = this.charts(tile, has);
    const base = !shown.length || shown.some((id) => stored.includes(id)) ? stored : shown;
    settings.setCharts(tile, change(base));
  }

  /** Is this chart on the tile (for the Customize chips)? */
  hasChart(tile: TileId, id: string): boolean {
    return this.charts(tile).includes(id);
  }

  toggleChart(tile: TileId, id: string) {
    this.editCharts(tile, (ids) => (ids.includes(id) ? removedChart(ids, id) : addedChart(ids, id)));
  }

  moveChart(tile: TileId, id: string, by: number) {
    this.editCharts(tile, (ids) => movedChart(ids, id, by));
  }

  clearCharts(tile: TileId) {
    settings.setCharts(tile, []);
  }

  /** The chart options this PC can show, in the tile's order, without "none". */
  private chartOptions(tile: TileId, has?: ChartHas) {
    return (DETAIL[tile]?.charts ?? []).filter(([id]) => id !== "none" && !this.why(tile, id) && (has?.(id) ?? true));
  }

  /**
   * The list behind a tile's header badge: what this PC can show, without "none" (a hidden badge would leave
   * nothing to click on). The choice is the same setting as in Settings > Modules > Customize.
   * Undefined when there is nothing to choose between.
   */
  quick(tile: TileId, kind: "badge"): QuickPick | undefined {
    const options = (DETAIL[tile]?.badges ?? [])
      .filter(([id]) => id !== "none" && !this.why(tile, id))
      .map(([id, key]) => ({ id, label: tr(key) }));
    if (options.length < 2) return undefined;
    return { options, value: this.badge(tile), set: (id) => settings.setBadge(tile, id), tip: tr("pickBadgeTip") };
  }

  /**
   * The list behind one chart's name: switch this chart to another one that isn't on the tile yet, or (when another
   * chart stays) take it away. Undefined when there is neither.
   */
  chartPick(tile: TileId, id: string, has?: ChartHas): QuickPick | undefined {
    const shown = this.charts(tile, has);
    const options = this.chartOptions(tile, has)
      .filter(([o]) => o === id || !shown.includes(o))
      .map(([o, key]) => ({ id: o, label: tr(key) }));
    const remove =
      shown.length > 1
        ? { label: tr("removeChart"), run: () => this.editCharts(tile, (ids) => removedChart(ids, id), has) }
        : undefined;
    if (options.length < 2 && !remove) return undefined;
    return {
      options,
      value: id,
      set: (to) => this.editCharts(tile, (ids) => replacedChart(ids, id, to), has),
      tip: tr("pickChartTip"),
      remove,
    };
  }

  /** The list behind "add a chart": what this PC can show and the tile doesn't yet. Undefined when that is nothing. */
  chartAdd(tile: TileId, has?: ChartHas): QuickPick | undefined {
    const shown = this.charts(tile, has);
    const options = this.chartOptions(tile, has)
      .filter(([o]) => !shown.includes(o))
      .map(([o, key]) => ({ id: o, label: tr(key) }));
    if (!options.length) return undefined;
    return { options, value: "", set: (to) => this.editCharts(tile, (ids) => addedChart(ids, to), has), tip: tr("addChartTip") };
  }

  private pick(tile: TileId, chosen: string, options?: [string, Key][]): string {
    if (!this.why(tile, chosen)) return chosen;
    return options?.find(([o]) => !this.why(tile, o))?.[0] ?? "none";
  }

  /** The options of a tile that can't be shown, grouped by reason, with their (translated) labels. */
  gone(tile: TileId, label: (k: Key) => string): { why: Key; labels: string[] }[] {
    const def = DETAIL[tile];
    if (!def) return [];
    const out: { why: Key; labels: string[] }[] = [];
    const done = new Set<string>();
    for (const [id, key] of [...def.rows, ...(def.badges ?? []), ...(def.charts ?? [])]) {
      const why = this.why(tile, id);
      if (!why || done.has(id)) continue;
      done.add(id);
      const group = out.find((g) => g.why === why);
      if (group) group.labels.push(label(key));
      else out.push({ why, labels: [label(key)] });
    }
    return out;
  }
}

export const caps = new Caps();
