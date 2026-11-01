import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** What the server offers (updater.rs `Offer`; the download address and checksum stay in Rust). */
export type UpdateOffer = {
  version: string;
  /** HTML or plain text, as the server wrote it: show it through `notesToText`, never as markup. */
  notes: string;
  size: number | null;
  mandatory: boolean;
};

/** Where the update stands (updatestate.rs `Phase`). `error` is a code such as "checksum" or "network: ..." (see UpdateView). */
export type UpdatePhase = { state: "idle" | "downloading" | "ready" | "installing" | "failed"; error?: string };

/**
 * The update on offer and what is happening to it, kept in step with the backend through events. One store for the popup
 * (pill, Settings line, update screen); the tray menu window reads the offer on its own.
 */
class UpdateStore {
  offer = $state<UpdateOffer | null>(null);
  phase = $state<UpdatePhase>({ state: "idle" });
  /** Bytes received / expected (0 = unknown) while downloading. */
  done = $state(0);
  total = $state(0);
  /** Bytes per second, smoothed. */
  speed = $state(0);
  private last: { t: number; done: number } | null = null;

  /** Reads the current state, follows the backend's events; returns the cleanup. */
  start(): () => void {
    const offs: Promise<UnlistenFn>[] = [
      listen<UpdateOffer | null>("update-offer", (e) => (this.offer = e.payload ?? null)),
      listen<UpdatePhase>("update-phase", (e) => this.setPhase(e.payload)),
      listen<{ done: number; total: number }>("update-progress", (e) => this.progress(e.payload.done, e.payload.total)),
    ];
    this.refresh();
    return () => offs.forEach((p) => p.then((f) => f()).catch(() => {}));
  }

  async refresh() {
    try {
      this.offer = await invoke<UpdateOffer | null>("get_update_offer");
      this.setPhase(await invoke<UpdatePhase>("get_update_phase"));
    } catch {
      /* no backend (a plain browser): nothing on offer */
    }
  }

  private setPhase(next: UpdatePhase) {
    if (next.state === "downloading" && this.phase.state !== "downloading") {
      this.done = 0;
      this.total = 0;
      this.speed = 0;
      this.last = null;
    }
    this.phase = next;
  }

  private progress(done: number, total: number) {
    const now = performance.now();
    if (this.last && done >= this.last.done && now > this.last.t) {
      const inst = ((done - this.last.done) / (now - this.last.t)) * 1000;
      this.speed = this.speed ? this.speed * 0.6 + inst * 0.4 : inst;
    }
    this.last = { t: now, done };
    this.done = done;
    this.total = total;
  }

  /** "Update": download (or join the running download), install and restart. Returns false when there is nothing to install. */
  async go(): Promise<boolean> {
    try {
      await invoke("start_update");
      return true;
    } catch {
      this.phase = { state: "failed", error: "no_offer" };
      return false;
    }
  }

  cancel() {
    invoke("cancel_update").catch(() => {});
  }
}

export const update = new UpdateStore();
