import type { Sample } from "./slots";

/**
 * The sample with a value nearest to `target` (viewBox x), at most `maxDist` away, or -1. `pts` are in x order (a
 * chart's samples always are), so this is a binary search plus a short walk past gaps: the pointer moving over a
 * chart, and every other chart mirroring it (hoversync), no longer scan all the samples on each move.
 * Of two equally near samples the earlier one wins (as the plain scan did).
 */
export function nearestIndex(pts: readonly Sample[], target: number, maxDist: number): number {
  let lo = 0;
  let hi = pts.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (pts[mid].x < target) lo = mid + 1;
    else hi = mid;
  }
  let best = -1;
  let dist = Infinity;
  for (let i = lo; i < pts.length; i++) {
    const d = pts[i].x - target;
    if (d > maxDist) break;
    if (pts[i].v != null) {
      best = i;
      dist = d;
      break;
    }
  }
  for (let i = lo - 1; i >= 0; i--) {
    const d = target - pts[i].x;
    if (d > maxDist || d > dist) break;
    if (pts[i].v != null) {
      best = i;
      break;
    }
  }
  return best;
}

/** Where moment `t` (ms) sits on a chart whose newest sample is `latest` and which shows `windowSecs`. */
export function xOfTime(t: number, latest: number, windowSecs: number, width: number): number {
  return (1 - (latest - t) / (windowSecs * 1000)) * width;
}
