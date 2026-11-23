/**
 * The moment under the pointer on one chart, mirrored on every other chart with a time axis (as in MSI Afterburner):
 * they show a dashed line, a dot and their own reading at that moment. `owner` is the chart the pointer is on; only
 * it shows the tooltip and asks who used the most then.
 */
export const hoverSync = $state<{ t: number | null; owner: object | null }>({ t: null, owner: null });

export function publishHover(owner: object, t: number | null): void {
  if (t != null) {
    hoverSync.owner = owner;
    hoverSync.t = t;
  } else if (hoverSync.owner === owner) {
    hoverSync.owner = null;
    hoverSync.t = null;
  }
}
