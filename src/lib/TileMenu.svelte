<script lang="ts">
  /**
   * A tile's right-click menu (rows and their order: tilemenu.ts). Same look as the process list's menu; closes on
   * Escape, a click elsewhere, scrolling, or when the popup loses focus. An action may answer with a short message
   * ("Copied"), shown in place for a moment before the menu closes.
   */
  import Icon from "./Icon.svelte";
  import { tr } from "./tr";
  import { tileMenuRows, type TileAction } from "./tilemenu";

  let {
    x,
    y,
    title,
    onaction,
    onclose,
  }: {
    x: number;
    y: number;
    title: string;
    onaction: (id: TileAction) => Promise<{ msg: string; bad?: boolean } | void> | { msg: string; bad?: boolean } | void;
    onclose: () => void;
  } = $props();

  const rows = tileMenuRows();
  let el = $state<HTMLElement>();
  let w = $state(0);
  let h = $state(0);
  let note = $state<{ msg: string; bad?: boolean } | null>(null);
  let closeTimer: ReturnType<typeof setTimeout> | undefined;

  const pos = $derived.by(() => {
    const margin = 8;
    const left = Math.min(x, window.innerWidth - (w || 190) - margin);
    const top = y + (h || 110) + margin > window.innerHeight ? y - (h || 110) : y;
    return { left: Math.max(margin, left), top: Math.max(margin, top) };
  });

  async function run(id: TileAction) {
    const r = await onaction(id);
    if (r && r.msg) {
      note = r;
      closeTimer = setTimeout(onclose, 1200);
    } else {
      onclose();
    }
  }
  function onWindowDown(e: PointerEvent) {
    if (el && !el.contains(e.target as Node)) onclose();
  }
  // Capture phase, so Escape closes the menu instead of hiding the whole popup; scrolling the tiles closes it too.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopImmediatePropagation();
      e.preventDefault();
      onclose();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("scroll", onclose, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("scroll", onclose, true);
      clearTimeout(closeTimer);
    };
  });
  /** The popup's card clips its content (and contains fixed elements): the menu lives on <body> instead. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
  $effect(() => {
    el?.querySelector<HTMLButtonElement>("button")?.focus({ preventScroll: true });
  });
</script>

<svelte:window onpointerdown={onWindowDown} onblur={onclose} />
<svelte:document onvisibilitychange={onclose} />

<div
  class="ctx"
  role="menu"
  tabindex="-1"
  use:portal
  bind:this={el}
  bind:clientWidth={w}
  bind:clientHeight={h}
  style="left:{pos.left}px;top:{pos.top}px"
>
  <p class="who">{title}</p>
  {#if note}
    <p class="msg" class:bad={note.bad}>{note.msg}</p>
  {:else}
    {#each rows as r, i (r.kind === "sep" ? `sep${i}` : r.id)}
      {#if r.kind === "sep"}
        <div class="sep" role="separator"></div>
      {:else}
        <button role="menuitem" data-id={r.id} style="--tone: var(--{r.tone})" onclick={() => run(r.id)}>
          <Icon name={r.icon} size={14} />
          <span>{tr(r.label)}</span>
        </button>
      {/if}
    {/each}
  {/if}
</div>

<style>
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
  .ctx:focus {
    outline: none;
  }
  .who {
    padding: 3px 8px 5px;
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    overflow-wrap: anywhere;
  }
  button {
    display: flex;
    align-items: center;
    gap: 8px;
    text-align: left;
    padding: 6px 8px;
    border-radius: 8px;
    font-size: 12px;
    color: var(--text);
  }
  button :global(svg) {
    flex: none;
    color: var(--tone);
  }
  button:hover,
  button:focus-visible {
    outline: none;
    background: color-mix(in srgb, var(--muted) 16%, transparent);
  }
  .sep {
    height: 1px;
    margin: 3px 8px;
    background: color-mix(in srgb, var(--lo) 55%, transparent);
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
