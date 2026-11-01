<script module lang="ts">
  /** Rows the list shows on its own, and at most when its column has room to spare (see `extra`). */
  export const PROC_ROWS = { classic: 3, advanced: 5, maxClassic: 6, maxAdvanced: 10 };
</script>

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import Tile from "../Tile.svelte";
  import { lower, tr } from "../tr";
  import { bytes, rate } from "../format";
  import type { Key } from "../i18n";
  import type { Metrics, Proc } from "../metrics.svelte";
  import { apps } from "../apps.svelte";
  import { settings } from "../settings.svelte";

  type Sort = "cpu" | "mem" | "disk" | "gpu";
  let {
    m,
    adv,
    span,
    extra = 0,
  }: {
    m: Metrics;
    adv: boolean;
    span: number;
    /** Rows that fit in what is left of the tile's column (the masonry works this out). */
    extra?: number;
  } = $props();
  // Settings > Modules > Customize (advanced view): opening tab, list length, rows and the header badge.
  const show = (row: string) => !adv || settings.shown("processes", row);
  const SORTS: Sort[] = ["cpu", "mem", "disk", "gpu"];
  function openingTab(): Sort {
    if (!adv) return "cpu";
    const pick = settings.pickOf("processes", "tab");
    const id = pick === "last" ? settings.procTab : pick;
    return SORTS.includes(id as Sort) ? (id as Sort) : "cpu";
  }
  let sort = $state<Sort>(openingTab());
  // Without the tab bar there is no way to switch: the list follows the chosen tab (also when changed in Settings).
  $effect(() => {
    if (!show("tabs")) sort = openingTab();
  });
  function choose(id: Sort) {
    sort = id;
    closeMenu();
    if (settings.procTab !== id) {
      settings.procTab = id;
      settings.save();
    }
  }
  const badgeKind = $derived(settings.badgeOf("processes"));
  const badge = $derived(
    !adv || badgeKind === "none"
      ? undefined
      : badgeKind === "threads"
        ? m.threadCount && `${m.threadCount} ${lower(tr("threadCount"))}`
        : m.processCount && `${m.processCount} ${lower(tr("procCount"))}`,
  );

  const tabs: [Sort, Key][] = [
    ["cpu", "sortCpu"],
    ["mem", "sortMem"],
    ["disk", "sortDisk"],
    ["gpu", "sortGpu"],
  ];
  const value = (p: Proc): number => ({ cpu: p.cpu, mem: p.mem, disk: p.disk, gpu: p.gpu })[sort];
  const amount = (p: Proc): string => {
    switch (sort) {
      case "mem":
        return bytes(p.mem);
      case "disk":
        return rate(p.disk);
      default:
        return `${value(p).toFixed(1)}%`;
    }
  };
  const list = $derived(
    { cpu: m.topCpu, mem: m.topMem, disk: m.topDisk, gpu: m.topGpu }[sort] ?? [],
  );
  /** A fixed list length (Customize) shows exactly that many and doesn't fill the column's spare room. */
  const fixed = $derived(adv ? Number(settings.pickOf("processes", "length")) || 0 : 0);
  const base = $derived(fixed || (adv ? PROC_ROWS.advanced : PROC_ROWS.classic));
  const most = $derived(fixed || (adv ? PROC_ROWS.maxAdvanced : PROC_ROWS.maxClassic));
  // Idle apps would only pad the lists with zeros (CPU / GPU below 0.05 % reads "0.0%").
  const shown = (p: Proc) => (sort === "cpu" || sort === "gpu" ? value(p) >= 0.05 : value(p) > 0);
  const rows = $derived(list.filter(shown).slice(0, Math.min(most, base + Math.max(0, extra))));
  const clean = (n: string) => n.replace(/\.exe$/i, "");

  $effect(() => apps.request(rows));

  // ---- right-click menu: open the file location, end the app (asks once more first)
  type Menu = { name: string; pids: number[]; count: number; x: number; y: number; confirm: boolean; msg: string; bad: boolean };
  let menu = $state<Menu | null>(null);
  let menuEl = $state<HTMLElement>();
  let menuW = $state(0);
  let menuH = $state(0);
  let closeTimer: ReturnType<typeof setTimeout> | undefined;
  const menuPos = $derived.by(() => {
    if (!menu) return { left: 0, top: 0 };
    const margin = 8;
    const left = Math.min(menu.x, window.innerWidth - (menuW || 200) - margin);
    const top = menu.y + (menuH || 90) + margin > window.innerHeight ? menu.y - (menuH || 90) : menu.y;
    return { left: Math.max(margin, left), top: Math.max(margin, top) };
  });

  function openMenu(e: MouseEvent, p: Proc) {
    e.preventDefault();
    clearTimeout(closeTimer);
    menu = { name: p.name, pids: [...p.pids], count: p.count, x: e.clientX, y: e.clientY, confirm: false, msg: "", bad: false };
  }
  function closeMenu() {
    clearTimeout(closeTimer);
    menu = null;
  }
  async function reveal() {
    if (!menu) return;
    const ok = await invoke<boolean>("reveal_app", { pids: menu.pids }).catch(() => false);
    if (ok) closeMenu();
    else if (menu) menu = { ...menu, msg: tr("appNoPath"), bad: true };
  }
  async function endTask() {
    if (!menu) return;
    if (!menu.confirm) {
      menu = { ...menu, confirm: true };
      return;
    }
    const target = menu;
    const r = await invoke<{ ended: number; failed: number; own: number }>("end_app", { name: target.name, pids: target.pids }).catch(
      () => ({ ended: 0, failed: target.pids.length, own: 0 }),
    );
    if (menu !== target && menu?.name !== target.name) return;
    const msg =
      r.ended > 0 && r.failed === 0 && r.own === 0
        ? tr("appEnded")
        : r.ended > 0
          ? tr("appEndedSome").replace("{n}", String(r.ended)).replace("{m}", String(r.ended + r.failed + r.own))
          : r.own > 0 && r.failed === 0
            ? tr("appEndOwn")
            : tr("appEndFailed");
    menu = { ...target, confirm: false, msg, bad: r.ended === 0 };
    closeTimer = setTimeout(closeMenu, 1600);
  }
  function onWindowDown(e: PointerEvent) {
    if (menu && menuEl && !menuEl.contains(e.target as Node)) closeMenu();
  }
  // Capture phase, so Escape closes the menu instead of hiding the whole popup; scrolling the tiles closes it too.
  const open = $derived(menu !== null);
  $effect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopImmediatePropagation();
      e.preventDefault();
      closeMenu();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("scroll", closeMenu, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("scroll", closeMenu, true);
    };
  });
  /** The popup's card clips its content (and contains fixed elements): the menu lives on <body> instead. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
  $effect(() => () => clearTimeout(closeTimer));
</script>

<!-- Not on resize: the popup re-fits its height whenever the list changes length. -->
<svelte:window onpointerdown={onWindowDown} onblur={closeMenu} />
<svelte:document onvisibilitychange={closeMenu} />

<Tile
  title={tr("processes")}
  icon="processes"
  color="var(--cpu)"
  {span}
  badge={badge || undefined}
>
  {#if show("tabs")}
    <div class="seg">
      {#each tabs as [id, key] (id)}
        <button class:on={sort === id} onclick={() => choose(id)}>{tr(key)}</button>
      {/each}
    </div>
  {:else}
    <!-- no tab bar: still say what the list is sorted by (CPU and GPU both read "12.3%") -->
    <p class="sub">{tr("sortedBy")}: {tr(tabs.find(([id]) => id === sort)?.[1] ?? "sortCpu")}</p>
  {/if}
  {#if rows.length === 0}
    <p class="sub">{tr("noData")}</p>
  {:else}
    {#each rows as p, i (p.name)}
      {@const info = apps.info[p.name]}
      <div
        class="row"
        class:menu-on={menu?.name === p.name}
        data-extra={i >= base ? "" : undefined}
        role="listitem"
        oncontextmenu={(e) => openMenu(e, p)}
      >
        <!-- the classic tile is narrow: the "×24" counter (it would break "chrome" in two) is only in the tooltip -->
        <span class="name" title={`${info?.path ?? p.name}${p.count > 1 ? ` (×${p.count})` : ""}`}>
          {#if !show("icons")}
            <!-- no icon -->
          {:else if info?.icon}
            <img class="ic" src={info.icon} alt="" width="16" height="16" />
          {:else}
            <span class="ic ph" aria-hidden="true">{clean(p.name).charAt(0).toUpperCase()}</span>
          {/if}
          <span class="n">{clean(p.name)}</span>{#if p.count > 1 && adv && show("multi")}<em>×{p.count}</em>{/if}
        </span>
        <b>{amount(p)}</b>
      </div>
    {/each}
  {/if}
</Tile>

{#if menu}
  <div
    class="ctx"
    role="menu"
    tabindex="-1"
    use:portal
    bind:this={menuEl}
    bind:clientWidth={menuW}
    bind:clientHeight={menuH}
    style="left:{menuPos.left}px;top:{menuPos.top}px"
  >
    <p class="who">{clean(menu.name)}{#if menu.count > 1}&nbsp;<em>×{menu.count}</em>{/if}</p>
    {#if menu.msg}
      <p class="msg" class:bad={menu.bad}>{menu.msg}</p>
    {:else}
      <button role="menuitem" disabled={apps.info[menu.name] != null && !apps.info[menu.name].path} onclick={reveal}>
        {tr("appReveal")}
      </button>
      <button role="menuitem" class="danger" class:confirm={menu.confirm} onclick={endTask}>
        {menu.confirm
          ? menu.count > 1
            ? tr("appEndConfirmN").replace("{n}", String(menu.count))
            : tr("appEndConfirm")
          : tr("appEnd")}
      </button>
    {/if}
  </div>
{/if}

<style>
  .seg {
    display: flex;
    padding: 3px;
    border-radius: 11px;
    box-shadow: var(--inset-sm);
    gap: 2px;
    align-self: stretch;
  }
  .seg button {
    flex: 1;
    padding: 3px 2px;
    border-radius: 8px;
    font-size: 10.5px;
    white-space: nowrap;
    color: var(--muted);
  }
  .seg button.on {
    box-shadow: var(--raised-sm);
    color: var(--text);
    font-weight: 600;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 10px;
    font-size: 12px;
    border-radius: 6px;
    margin: 0 -4px;
    padding: 0 4px;
  }
  .row.menu-on {
    background: color-mix(in srgb, var(--muted) 14%, transparent);
  }
  /* Uzun uygulama adı en çok 2 satıra bölünür; "×9" sayacı her zaman görünür kalır (eskiden adla birlikte kırpılıyordu). */
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex: 1;
  }
  .ic {
    flex: none;
    width: 16px;
    height: 16px;
  }
  .ph {
    display: grid;
    place-items: center;
    border-radius: 5px;
    box-shadow: var(--inset-sm);
    font-size: 9px;
    font-weight: 700;
    color: var(--muted);
  }
  .n {
    min-width: 0;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }
  em {
    flex: none;
    font-style: normal;
    font-size: 10.5px;
    color: var(--muted);
  }
  b {
    font-weight: 650;
    flex: none;
  }
  .ctx {
    position: fixed;
    z-index: 90;
    min-width: 170px;
    max-width: 240px;
    padding: 6px;
    border-radius: 12px;
    background: var(--bg);
    box-shadow:
      var(--raised),
      0 0 0 1px color-mix(in srgb, var(--lo) 55%, transparent);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .who {
    padding: 3px 8px 5px;
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    overflow-wrap: anywhere;
  }
  .who em {
    font-size: 10.5px;
  }
  .ctx button {
    text-align: left;
    padding: 6px 8px;
    border-radius: 8px;
    font-size: 12px;
    color: var(--text);
  }
  .ctx button:hover:not(:disabled),
  .ctx button:focus-visible {
    background: color-mix(in srgb, var(--muted) 16%, transparent);
  }
  .ctx button:disabled {
    color: var(--muted);
    opacity: 0.6;
  }
  .ctx button.danger {
    color: var(--bad-text);
  }
  .ctx button.confirm {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    font-weight: 600;
  }
  .msg {
    padding: 6px 8px;
    font-size: 12px;
    color: var(--ok-text);
  }
  .msg.bad {
    color: var(--bad-text);
  }
</style>
