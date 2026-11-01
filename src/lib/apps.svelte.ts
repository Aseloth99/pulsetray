import { invoke } from "@tauri-apps/api/core";
import type { Proc } from "./metrics.svelte";

export interface AppInfo {
  name: string;
  /** The exe; null for protected / system processes (vmmem, System...). */
  path: string | null;
  /** PNG data URL, or null when the exe has no icon. */
  icon: string | null;
}

/** Icons and paths of the apps the list has shown, by process name (asked for once per name). */
class Apps {
  info = $state.raw<Record<string, AppInfo>>({});
  private pending = new Set<string>();

  /** Fetch what is still unknown for these apps (a few pids each: the first may have exited already). */
  request(list: Proc[]) {
    const missing = list.filter((p) => !(p.name in this.info) && !this.pending.has(p.name) && p.pids?.length);
    if (!missing.length) return;
    for (const p of missing) this.pending.add(p.name);
    invoke<AppInfo[]>("app_infos", { apps: missing.map((p) => [p.name, p.pids.slice(0, 4)]) })
      .then((found) => {
        const next = { ...this.info };
        for (const a of found) next[a.name] = a;
        this.info = next;
      })
      .catch(() => {})
      .finally(() => {
        for (const p of missing) this.pending.delete(p.name);
      });
  }
}

export const apps = new Apps();
