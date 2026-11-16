import test from "node:test";
import assert from "node:assert/strict";
import { lib, read } from "./load.mjs";

const d = await lib("disks");
const disk = (index, letters, extra = {}) => ({ index, letters, active: 0, read: 0, write: 0, ...extra });

test("a mount point becomes the drive letter the counters use", () => {
  assert.equal(d.mountLetter("C:\\"), "C:");
  assert.equal(d.mountLetter("d:/"), "D:");
  assert.equal(d.mountLetter("E:"), "E:");
  assert.equal(d.mountLetter("C:\\Mounts\\Backup"), "C:\\Mounts\\Backup", "a mounted folder is not a drive letter");
  assert.equal(d.mountLetter("/"), "/");
});

test("a disk owns the volumes whose letters it holds", () => {
  const two = disk(1, "D: E:");
  assert.equal(d.ownsMount(two, "D:\\"), true);
  assert.equal(d.ownsMount(two, "E:\\"), true);
  assert.equal(d.ownsMount(two, "C:\\"), false);
  assert.equal(d.ownsMount(disk(2, ""), "C:\\"), false, "a disk without letters owns nothing");
  assert.equal(d.ownsMount(disk(0, "C:"), "C:\\Mounts\\Backup"), false, "a mounted folder is not matched by its drive");
  assert.equal(d.ownsMount(disk(3, "C:"), "c:/"), true, "case and slash style don't matter");
});

test("tab and full names: letters first, then the model, then 'Disk n'", () => {
  assert.equal(d.tabLabel(disk(0, "C:"), "Samsung SSD 980 PRO 1TB", "Disk"), "C:");
  assert.equal(d.tabLabel(disk(2, ""), "WD Black", "Disk"), "WD Black");
  assert.equal(d.tabLabel(disk(4, ""), undefined, "Disk"), "Disk 4");
  assert.equal(d.fullLabel(disk(0, "C:"), "Samsung SSD 980 PRO 1TB", "Disk"), "C: · Samsung SSD 980 PRO 1TB");
  assert.equal(d.fullLabel(disk(0, "C:"), undefined, "Disk"), "C:");
  assert.equal(d.fullLabel(disk(2, ""), "WD Black", "Disk"), "WD Black");
  assert.equal(d.fullLabel(disk(5, ""), undefined, "Sürücü"), "Sürücü 5");
});

test("series of one disk and of all of them", () => {
  assert.equal(d.seriesPrefix(undefined), "disk.");
  assert.equal(d.seriesPrefix(disk(3, "F:")), "disk.3.");
  assert.notEqual(`${d.seriesPrefix(disk(0, "C:"))}active`, "disk.active");
});

test("up to four disks are tabs, more become a list", () => {
  assert.equal(d.MAX_TABS, 4);
});

test("the disk tile and its customisation hang together", () => {
  const tile = read("src/lib/tiles/DiskTile.svelte");
  assert.match(tile, /seriesPrefix\(sel\)/, "the charts follow the chosen disk");
  assert.match(tile, /who=\{sel \? undefined : "disk"\}/, "a chart of one disk doesn't name apps (they are not tied to a disk)");
  assert.match(tile, /phys\.length <= MAX_TABS/);
  const detail = read("src/lib/tiledetail.ts");
  const diskBlock = detail.slice(detail.indexOf("  disk: {"), detail.indexOf("  battery: {"));
  assert.match(diskBlock, /\["perDisk", "perDisk"\]/);
  assert.match(diskBlock, /\["tabs", "diskTabs"\]/);
  assert.match(diskBlock, /charts: \[\s*\["active", "active"\]/, "Active is the default chart");
});

test("the series of every disk are recorded by the UI under the names the backend uses", () => {
  const store = read("src/lib/metrics.svelte.ts");
  assert.match(store, /disk\.\$\{p\.index\}\.active/);
  const rust = read("src-tauri/src/history.rs");
  assert.match(rust, /format!\("disk\.\{idx\}\.\{metric\}"\)/);
  for (const metric of ["active", "read", "write"]) assert.match(rust, new RegExp(`put\\("${metric}"`), `the backend records ${metric}`);
  assert.match(store, /disk\.\$\{p\.index\}\.read/);
  assert.match(store, /disk\.\$\{p\.index\}\.write/);
});

test("the Disk tile remembers its tab by drive letters, so it survives a restart and a USB disk coming or going", () => {
  const c = disk(0, "C:");
  const f = disk(1, "F:");
  const blank = disk(2, "");
  assert.equal(d.diskKey(c), "letters:C:");
  assert.equal(d.diskKey(blank), "index:2", "a disk without letters is remembered by its number");
  assert.equal(d.findDisk([c, f], "letters:C:"), c);
  assert.equal(d.findDisk([disk(3, "F:"), disk(1, "C:")], "letters:C:")?.index, 1, "numbers changed, letters did not");
  assert.equal(d.findDisk([disk(0, "C: D:")], "letters:C:")?.letters, "C: D:", "a volume added to the disk");
  assert.equal(d.findDisk([c, f], "letters:G:"), undefined, "a disk that is gone shows all disks");
  assert.equal(d.findDisk([c, blank], "index:2"), blank);
  assert.equal(d.findDisk([c], null), undefined);
});

test("the chosen disk tab is kept in the settings", () => {
  const tile = read("src/lib/tiles/DiskTile.svelte");
  assert.match(tile, /findDisk\(phys, settings\.diskTab\)/);
  assert.match(tile, /settings\.diskTab = key;\s+settings\.save\(\);/);
  assert.doesNotMatch(tile, /let chosen = \$state/, "a plain component state was forgotten on every restart");
  const s = read("src/lib/settings.svelte.ts");
  assert.match(s, /diskTab: this\.diskTab,/, "saved");
  assert.match(s, /this\.diskTab = s\.diskTab;/, "loaded");
});
