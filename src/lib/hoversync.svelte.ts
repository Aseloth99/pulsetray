/**
 * The moment under the pointer on one chart, mirrored on every other chart with a time axis (as in MSI Afterburner):
 * they show a dashed line, a dot and their own reading at that moment. `owner` is the chart the pointer is on; only
 * it shows the tooltip and asks who used the most then.
 */
class HoverSync {
  t = $state<number | null>(null);
  /** Raw, not deep state: a deep `$state` stores a proxy of the chart's object, `===` against the chart's own object
   *  never matches, and the owner could never clear the moment (every chart kept a dashed line after the pointer left). */
  owner = $state.raw<object | null>(null);
}

export const hoverSync = new HoverSync();

export function publishHover(owner: object, t: number | null): void {
  if (t != null) {
    hoverSync.owner = owner;
    hoverSync.t = t;
  } else if (hoverSync.owner === owner) {
    hoverSync.owner = null;
    hoverSync.t = null;
  }
}
