/** A sample on a chart's time axis: `x` in viewBox units, `t` in ms, `gap`: the line breaks before it. */
export type Sample<V = number | null> = { i: number; x: number; v: V; t: number | null; gap: boolean };

/**
 * One point per time slot (the slot's highest value, so a short spike stays visible) for a window with more samples
 * than `maxPoints`. Slots are cut at fixed clock times, not counted back from the newest sample: a sample then stays
 * in the same slot as the window slides, so only the newest slot changes from one second to the next. Counting back
 * from the newest sample re-dealt every sample into new slots on each tick and the whole line shimmered.
 */
export function slotSamples<V extends number | null>(
  raw: Sample<V>[],
  o: { width: number; windowSecs: number; maxPoints: number; gapMs: number; latest: number },
): Sample<V>[] {
  if (raw.length <= o.maxPoints || raw[0].t == null) return raw;
  const span = o.windowSecs * 1000;
  const slot = span / o.maxPoints;
  const xOf = (center: number) => Math.min(o.width, Math.max(0, (1 - (o.latest - center) / span) * o.width));
  const out: Sample<V>[] = [];
  let cur: Sample<V> | null = null;
  let curSlot = NaN;
  let lastT: number | null = null;
  for (const p of raw) {
    const s = Math.floor((p.t as number) / slot);
    if (s !== curSlot || p.gap) {
      if (cur) out.push(cur);
      cur = { ...p, x: xOf((s + 0.5) * slot), gap: lastT != null && (p.t as number) - lastT > o.gapMs };
      curSlot = s;
    } else if (p.v != null && (cur!.v == null || p.v >= (cur!.v as number))) {
      cur = { ...cur!, v: p.v, t: p.t, i: p.i };
    }
    lastT = p.t;
  }
  if (cur) out.push(cur);
  return out;
}
