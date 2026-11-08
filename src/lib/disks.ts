/** What the Disk tile needs to tell the physical drives apart (the backend measures each one, see pdh.rs). */
export interface PhysDisk {
  /** The Windows disk number ("Disk 0"). */
  index: number;
  /** The drive letters on it, "C:" or "D: E:"; empty for a disk without a lettered volume. */
  letters: string;
  /** Active time (0..100), bytes read and written per second. */
  active: number;
  read: number;
  write: number;
}

/** More than this many drives (plus "All") don't fit as tabs: the choice becomes a list. */
export const MAX_TABS = 4;

/** `C:\` -> `C:` (the form the counters use for a drive letter); anything else (a mounted folder) as it is. */
export function mountLetter(mount: string): string {
  return /^[A-Za-z]:[\\/]?$/.test(mount) ? mount.slice(0, 2).toUpperCase() : mount;
}

/** Does this physical disk hold the volume mounted at `mount`? */
export function ownsMount(p: PhysDisk, mount: string): boolean {
  return p.letters.split(/\s+/).filter(Boolean).includes(mountLetter(mount));
}

/** A short name for a tab: its drive letters, else the model, else "Disk n". */
export function tabLabel(p: PhysDisk, model: string | undefined, fallback: string): string {
  return p.letters || model || `${fallback} ${p.index}`;
}

/** The whole name: letters and model ("C: · Samsung SSD 980 PRO 1TB"). */
export function fullLabel(p: PhysDisk, model: string | undefined, fallback: string): string {
  return [p.letters, model].filter(Boolean).join(" · ") || `${fallback} ${p.index}`;
}

/** The chart series of one disk, or of all of them (`undefined`): "disk." or "disk.<n>.". */
export function seriesPrefix(p: PhysDisk | undefined): string {
  return p ? `disk.${p.index}.` : "disk.";
}
