/**
 * The webview does not always say when the pointer leaves the window: moved off quickly (onto the taskbar, another
 * window, the desktop), the chart never gets `pointerleave`, and its hover line, dot and readout stay on screen.
 * While something shows a hover state it registers here, and the pointer's real position is checked a few times a
 * second; once it is outside the window, every waiting hover is cleared. Nothing is polled while nothing is hovered.
 */

/** True while the pointer is over the window; throws or gives null where that cannot be asked (a plain browser). */
export type Probe = () => Promise<boolean | null>;

export class LeaveGuard {
  private readonly waiting = new Set<() => void>();
  private timer: ReturnType<typeof setInterval> | undefined;
  private busy = false;
  private unsupported = false;
  private readonly probe: Probe;
  private readonly every: number;

  constructor(probe: Probe, every = 250) {
    this.probe = probe;
    this.every = every;
  }

  /** `onLeave` runs once when the pointer is found outside the window; the returned function cancels it. */
  watch(onLeave: () => void): () => void {
    if (this.unsupported) return () => {};
    this.waiting.add(onLeave);
    this.timer ??= setInterval(() => void this.check(), this.every);
    return () => {
      this.waiting.delete(onLeave);
      if (!this.waiting.size) this.stop();
    };
  }

  get watching(): number {
    return this.waiting.size;
  }

  async check(): Promise<void> {
    if (this.busy || !this.waiting.size) return;
    this.busy = true;
    let inside: boolean | null;
    try {
      inside = await this.probe();
    } catch {
      inside = null;
    } finally {
      this.busy = false;
    }
    if (inside === null) {
      // Not inside the app (the site preview, a test page): the DOM events are all there is.
      this.unsupported = true;
      this.waiting.clear();
      this.stop();
      return;
    }
    if (inside) return;
    const leave = [...this.waiting];
    this.waiting.clear();
    this.stop();
    for (const f of leave) f();
  }

  private stop(): void {
    clearInterval(this.timer);
    this.timer = undefined;
  }
}

/** Whether a point (physical pixels) is inside a rectangle (physical position + size); null for a bad reading. */
export function pointInside(
  c: { x: number; y: number },
  pos: { x: number; y: number },
  size: { width: number; height: number },
): boolean | null {
  const nums = [c.x, c.y, pos.x, pos.y, size.width, size.height];
  if (!nums.every((n) => typeof n === "number" && Number.isFinite(n)) || size.width <= 0 || size.height <= 0) return null;
  return c.x >= pos.x && c.y >= pos.y && c.x < pos.x + size.width && c.y < pos.y + size.height;
}

async function pointerInWindow(): Promise<boolean | null> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return null;
  const { cursorPosition, getCurrentWindow } = await import("@tauri-apps/api/window");
  const win = getCurrentWindow();
  const [c, pos, size] = await Promise.all([cursorPosition(), win.innerPosition(), win.innerSize()]);
  return pointInside(c, pos, size);
}

export const leaveGuard = new LeaveGuard(pointerInWindow);

/** Clears a hover state (`onLeave`) when the pointer leaves the window without the webview noticing. */
export const watchLeave = (onLeave: () => void): (() => void) => leaveGuard.watch(onLeave);
