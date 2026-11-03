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

  /** Under the trigger, flipped above when it would fall off the bottom, kept inside the window. */
  $effect(() => {
    if (!open || !menu || !anchor) return;
    const r = anchor.getBoundingClientRect();
    const w = menu.offsetWidth;
    const h = menu.offsetHeight;
    const margin = 8;
    const left = align === "end" ? r.right - w : r.left;
    let top = r.bottom + 4;
    if (top + h > window.innerHeight - margin) top = Math.max(margin, r.top - h - 4);
    pos = { left: Math.min(Math.max(left, margin), Math.max(margin, window.innerWidth - w - margin)), top };
    // Focus goes to the current choice once the list is visible (a hidden element can't take focus).
    tick().then(() => menu?.querySelector<HTMLElement>("[aria-checked=true]")?.focus());
  });

  // A list left open must not stay put when the content under it moves or the popup goes away.
  $effect(() => {
    if (!open) return;
    const away = (e: Event) => {
      if (!menu?.contains(e.target as Node) && !anchor?.contains(e.target as Node)) close();
    };
    const hide = () => close();
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("scroll", hide, true);
    document.addEventListener("visibilitychange", hide);
    window.addEventListener("blur", hide);
    window.addEventListener("resize", hide);
    return () => {
      document.removeEventListener("pointerdown", away, true);
      document.removeEventListener("scroll", hide, true);
      document.removeEventListener("visibilitychange", hide);
      window.removeEventListener("blur", hide);
      window.removeEventListener("resize", hide);
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
  </div>
{/if}

<style>
  .menu {
    position: fixed;
    z-index: 90;
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
</style>
