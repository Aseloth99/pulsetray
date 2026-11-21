import { untrack } from "svelte";
import { Tween, prefersReducedMotion } from "svelte/motion";
import { cubicOut } from "svelte/easing";

/** How long a gauge / bar / chart scale takes to reach a new reading (readings come about once a second). */
export const VALUE_MS = 300;

/**
 * A duration that honours Windows' "Show animations" switch (prefers-reduced-motion): 0 when the user asked for less
 * motion, so every tween and transition below collapses to an instant change.
 */
export const ms = (d: number): number => (prefersReducedMotion.current ? 0 : d);

/**
 * A number that glides to each new value of `fn` (cubic-out, `VALUE_MS`) instead of jumping; instant with reduced
 * motion. Call it during component initialisation. A value that is not a finite number shows as 0.
 */
export function smooth(fn: () => number, duration = VALUE_MS): Tween<number> {
  const read = () => {
    const v = fn();
    return Number.isFinite(v) ? v : 0;
  };
  const t = new Tween(read(), { easing: cubicOut });
  $effect.pre(() => {
    const v = read();
    const d = ms(duration);
    untrack(() => {
      if (v !== t.target) t.set(v, { duration: d });
    });
  });
  return t;
}
