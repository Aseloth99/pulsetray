<script lang="ts">
  import { tick, type Snippet } from "svelte";
  import type { QuickPick } from "./tiledetail";

  let {
    pick,
    align = "start",
    trigger,
  }: {
    pick: QuickPick;
    /** Which edge of the trigger the list lines up with. */
    align?: "start" | "end";
    /** The button that opens the list: it calls `toggle` from its click handler (the list opens at that button). */
    trigger: Snippet<[{ toggle: (e: MouseEvent) => void; open: boolean }]>;
  } = $props();

  let open = $state(false);
  let anchor: HTMLElement | null = null;
  let menu = $state<HTMLElement>();
  let pos = $state<{ left: number; top: number } | null>(null);

  function toggle(e: MouseEvent) {
    if (open) return close();
    anchor = e.currentTarget as HTMLElement;
    pos = null;
    open = true;
  }

  function close(refocus = false) {
    open = false;
    if (refocus) anchor?.focus();
  }

  function choose(id: string) {
    if (id !== pick.value) pick.set(id);
    close(true);
  }

  function place() {
    if (!menu || !anchor) return;
    const r = anchor.getBoundingClientRect();
    const w = menu.offsetWidth;
    const h = menu.offsetHeight;
    const margin = 8;
    const left = align === "end" ? r.right - w : r.left;
    let top = r.bottom + 4;
    if (top + h > window.innerHeight - margin) top = Math.max(margin, r.top - h - 4);
    pos = { left: Math.min(Math.max(left, margin), Math.max(margin, window.innerWidth - w - margin)), top };
  }

  /** Under the trigger, flipped above when it would fall off the bottom, kept inside the window. */
  $effect(() => {
    if (!open || !menu || !anchor) return;
    // In the top layer the list can't be clipped or moved by a card around its button (a transform, a filter, an
    // overflow); where the browser lacks popovers it stays a plain fixed element.
    try {
      if (!menu.matches(":popover-open")) menu.showPopover();
    } catch {}
    place();
    // Focus goes to the current choice once the list is visible (a hidden element can't take focus).
    tick().then(() => (menu?.querySelector<HTMLElement>("[aria-checked=true]") ?? menu?.querySelector<HTMLElement>("button"))?.focus());
  });

  // A list left open must not stay put when the popup goes away. A scroll or a resize (the popup re-fits its height
  // as the numbers change) is no reason to close it: it follows its button, and goes only when the button is no
  // longer in view.
  $effect(() => {
    if (!open) return;
    const away = (e: Event) => {
      if (!menu?.contains(e.target as Node) && !anchor?.contains(e.target as Node)) close();
    };
    // Focus can flicker right as the list opens (the click itself moves it); only a blur that lasts closes the list.
    const hide = () => close();
    const lost = () => setTimeout(() => !document.hasFocus() && close(), 250);
    const follow = () => {
      if (!anchor?.isConnected) return close();
      const r = anchor.getBoundingClientRect();
      if (r.bottom < 0 || r.top > window.innerHeight) return close();
      place();
    };
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("scroll", follow, true);
    document.addEventListener("visibilitychange", hide);
    window.addEventListener("blur", lost);
    window.addEventListener("resize", follow);
    return () => {
      document.removeEventListener("pointerdown", away, true);
      document.removeEventListener("scroll", follow, true);
      document.removeEventListener("visibilitychange", hide);
      window.removeEventListener("blur", lost);
      window.removeEventListener("resize", follow);
    };
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      return close(true);
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...(menu?.querySelectorAll<HTMLElement>("button") ?? [])];
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next = (at + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
    items[next]?.focus();
  }
</script>

{@render trigger({ toggle, open })}

{#if open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="menu"
    role="menu"
    tabindex="-1"
    popover="manual"
    bind:this={menu}
    onkeydown={onKey}
    style="left:{pos?.left ?? 0}px;top:{pos?.top ?? 0}px;visibility:{pos ? 'visible' : 'hidden'}"
  >
    {#each pick.options as o (o.id)}
      <button
        type="button"
        role="menuitemradio"
        aria-checked={o.id === pick.value}
        class:on={o.id === pick.value}
        onclick={() => choose(o.id)}
      >
        {o.label}
      </button>
    {/each}
    {#if pick.remove}
      {@const rm = pick.remove}
      <hr />
      <button
        type="button"
        role="menuitem"
        class="rm"
        onclick={() => {
          rm.run();
          close();
        }}>{rm.label}</button
      >
    {/if}
  </div>
{/if}

<style>
  .menu {
    position: fixed;
    z-index: 90;
    margin: 0;
    inset: auto;
    border: 0;
    overflow: visible;
    color: inherit;
    min-width: 120px;
    max-width: 240px;
    padding: 5px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-radius: 12px;
    background: var(--bg);
    box-shadow:
      var(--raised),
      0 0 0 1px color-mix(in srgb, var(--lo) 55%, transparent);
  }
  button {
    border: 0;
    background: transparent;
    font: inherit;
    font-size: 11.5px;
    color: var(--muted);
    text-align: left;
    white-space: nowrap;
    padding: 6px 10px;
    border-radius: 8px;
    cursor: pointer;
  }
  button:hover,
  button:focus-visible {
    color: var(--text);
    outline: none;
    box-shadow: var(--inset-sm);
  }
  button.on {
    color: var(--text);
    font-weight: 650;
    box-shadow: var(--inset-sm);
  }
  hr {
    border: 0;
    height: 1px;
    margin: 3px 6px;
    background: color-mix(in srgb, var(--lo) 55%, transparent);
  }
</style>
