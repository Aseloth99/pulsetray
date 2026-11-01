import type { Key } from "./i18n";
import type { Metrics } from "./metrics.svelte";
import { store } from "./metrics.svelte";
import { settings, type TileId } from "./settings.svelte";
import { DETAIL } from "./tiledetail";

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

  chart(tile: TileId): string {
    return this.pick(tile, settings.chartOf(tile), DETAIL[tile]?.charts);
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
