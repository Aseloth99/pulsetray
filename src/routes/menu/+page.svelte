<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import "../../app.css";
  import "../../glass.css";
  import Icon from "$lib/Icon.svelte";
  import { tr } from "$lib/tr";
  import { settings, applyStyle } from "$lib/settings.svelte";
  import { menuRows, type MenuRow } from "$lib/traymenu";

  /**
   * The tray icon's right-click menu. Windows' own menu can't follow the app's look, so Rust shows this small
   * window at the pointer instead (see `show_menu` in lib.rs). It lives in its own web view, reads the look the
   * main window saved, and re-reads it every time it is shown.
   */
  const PAD = 12; // transparent frame around the card (room for its shadow); same number as MENU_PAD in lib.rs

  /** The update on offer (null = none): the menu asks every time it is shown and hears about changes while it is hidden. */
  let offerVersion = $state<string | null>(null);
  /** A pinned panel is open next to the menu: the first entry then hides it instead of showing it. */
  let popupOpen = $state(false);
  /** The Theme row's list (System, Dark, Light) is open: it opens in place, collapsed again every time the menu is shown. */
  let themeOpen = $state(false);
  // The order lives in $lib/traymenu.ts (and is tested there): Update · Show/Hide · Settings · Theme ▸ · Mini Gauge · Exit.
  const rows: MenuRow[] = $derived(
    menuRows({ offer: offerVersion, popupOpen, theme: settings.theme, themeOpen, hudOn: settings.hud.on }),
  );
  const labelOf = (r: Extract<MenuRow, { kind: "item" }>) =>
    r.version ? tr(r.label).replace("{v}", r.version) : tr(r.label);
  function readOffer() {
    invoke<{ version: string } | null>("get_update_offer")
      .then((o) => (offerVersion = o?.version ?? null))
      .catch(() => {});
  }
  function readState() {
    invoke<{ popupOpen: boolean } | null>("tray_menu_state")
      .then((st) => (popupOpen = st?.popupOpen === true))
      .catch(() => {});
  }

  let systemDark = $state(false);
  const theme = $derived(settings.theme === "system" ? (systemDark ? "dark" : "light") : settings.theme);
  $effect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.lang = settings.lang;
  });
  $effect(() => {
    applyStyle(settings.style); // read again by settings.refresh() every time the menu is shown
  });

  /**
   * The web view remembers where the pointer last was inside it, so after the window is shown again the entry that
   * was under it then would look selected. Entries therefore only react once the pointer has really moved since
   * the menu opened.
   */
  let armed = $state(false);

  let shell = $state<HTMLElement>();
  /** Tells Rust how big the window must be: the card plus its transparent frame. */
  async function reportSize() {
    await tick();
    if (!shell) return;
    invoke("set_menu_size", {
      width: Math.ceil(shell.offsetWidth + 2 * PAD),
      height: Math.ceil(shell.offsetHeight + 2 * PAD),
    }).catch(() => {});
  }
  $effect(() => {
    settings.lang; // another language can change the text widths
    offerVersion; // and the update entry changes the card's size
    themeOpen; // and so does the open theme list
    reportSize();
  });

  /** The Theme row opens / closes its list; every other row asks Rust to do something (which also closes the menu). */
  function choose(r: Extract<MenuRow, { kind: "item" }>) {
    if (r.expandable) {
      themeOpen = !themeOpen;
      return;
    }
    invoke("tray_menu_action", { id: r.id }).catch(() => {});
  }
  const dismiss = () => invoke("hide_popup").catch(() => {});

  /** Arrow keys walk through the entries (a native menu does too; Right / Left open and close the theme list); Escape closes. */
  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") return void dismiss();
    if ((e.key === "ArrowRight" || e.key === "ArrowLeft") && (document.activeElement as HTMLElement | null)?.dataset.id === "theme") {
      e.preventDefault();
      themeOpen = e.key === "ArrowRight";
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...document.querySelectorAll<HTMLElement>(".item")];
    const at = items.indexOf(document.activeElement as HTMLElement);
    const step = e.key === "ArrowDown" ? 1 : -1;
    items[at < 0 ? (step > 0 ? 0 : items.length - 1) : (at + step + items.length) % items.length]?.focus();
  }

  /** A click on the transparent frame is a click "outside": the menu closes. */
  function onDown(e: MouseEvent) {
    if (!(e.target as HTMLElement | null)?.closest(".shell")) dismiss();
  }

  onMount(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    systemDark = mq.matches;
    const onMq = (e: MediaQueryListEvent) => (systemDark = e.matches);
    mq.addEventListener("change", onMq);
    readOffer();
    readState();
    // An offer that arrives (or goes) while the menu is hidden: the size is reported already, so the next opening fits.
    const unOffer = listen<{ version: string } | null>("update-offer", (e) => (offerVersion = e.payload?.version ?? null));
    const unShown = listen("menu-shown", () => {
      armed = false;
      themeOpen = false;
      readOffer();
      readState();
      settings.refresh();
      systemDark = mq.matches;
      (document.activeElement as HTMLElement | null)?.blur?.();
      reportSize();
    });
    return () => {
      mq.removeEventListener("change", onMq);
      unShown.then((f) => f());
      unOffer.then((f) => f());
    };
  });
</script>

<svelte:window
  onkeydown={onKey}
  onmousedown={onDown}
  onmousemove={() => (armed = true)}
  oncontextmenu={(e) => e.preventDefault()}
/>

<div class="shell" class:armed role="menu" bind:this={shell}>
  {#each rows as r, i (r.kind === "sep" ? `sep${i}` : r.id)}
    {#if r.kind === "sep"}
      <div class="sep" role="separator"></div>
    {:else}
      <button
        class="item"
        class:danger={r.danger}
        class:strong={r.strong}
        class:sub={r.sub}
        class:open={r.expanded}
        data-id={r.id}
        role={r.sub ? "menuitemradio" : r.id === "hud" ? "menuitemcheckbox" : "menuitem"}
        aria-checked={r.sub || r.id === "hud" ? r.check === true : undefined}
        aria-haspopup={r.expandable ? "menu" : undefined}
        aria-expanded={r.expandable ? r.expanded === true : undefined}
        onclick={() => choose(r)}
      >
        <Icon name={r.icon} size={r.sub ? 14 : 16} />
        <span class="label">{labelOf(r)}</span>
        {#if r.check}<span class="check" aria-hidden="true">✓</span>{/if}
        {#if r.expandable}<span class="chev" aria-hidden="true"><Icon name="next" size={14} /></span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .shell {
    margin: 12px; /* PAD */
    /* As wide as its longest entry (a long "Update · v10.12.3" must not be cut), never narrower than the usual card. */
    width: max-content;
    min-width: 172px;
    max-width: 320px;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: var(--bg);
    border-radius: 16px;
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--lo) 45%, transparent),
      0 6px 20px rgba(0, 0, 0, 0.28);
  }
  .item {
    pointer-events: none;
    display: flex;
    align-items: center;
    gap: 10px;
    height: 34px;
    padding: 0 10px;
    border-radius: 10px;
    background: transparent;
    font-size: 13px;
    text-align: left;
    transition: background 0.12s;
  }
  .shell.armed .item {
    pointer-events: auto;
  }
  .label {
    flex: 1;
    white-space: nowrap;
  }
  .item.strong {
    font-weight: 700;
    color: var(--net-text);
    background: color-mix(in srgb, var(--net) 12%, transparent);
  }
  .item.strong :global(svg) {
    color: var(--net-text);
  }
  .check {
    font-weight: 700;
    color: var(--net-text);
  }
  /* the Theme row's list opens in place: its choices are indented under it */
  .item.sub {
    height: 30px;
    padding-left: 28px;
    font-size: 12.5px;
  }
  .chev {
    flex: none;
    display: grid;
    color: var(--muted);
    transition: transform 0.15s;
  }
  .item.open .chev {
    transform: rotate(90deg);
  }
  .item :global(svg) {
    flex: none;
    color: var(--muted);
    transition: color 0.12s;
  }
  .item:hover,
  .item:focus-visible {
    outline: none;
    background: color-mix(in srgb, var(--net) 16%, transparent);
  }
  .item:hover :global(svg),
  .item:focus-visible :global(svg) {
    color: var(--net-text);
  }
  .item.danger:hover,
  .item.danger:focus-visible {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
  }
  .item.danger:hover :global(svg),
  .item.danger:focus-visible :global(svg) {
    color: var(--bad-text);
  }
  .item:active {
    box-shadow: var(--inset-sm);
  }
  .sep {
    height: 1px;
    margin: 4px 8px;
    background: color-mix(in srgb, var(--lo) 55%, transparent);
  }
</style>
