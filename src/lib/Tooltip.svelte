<script lang="ts">
  import { tooltip } from "./tipstate.svelte";

  let vw = $state(0);
  let vh = $state(0);
  let w = $state(0);
  let h = $state(0);

  /** Below the element by default, flipped above when it would fall off the bottom; kept inside the window. */
  const pos = $derived.by(() => {
    const r = tooltip.rect;
    const margin = 12;
    const bw = w || 220;
    const bh = h || 40;
    const left = Math.min(Math.max(r.left + r.width / 2 - bw / 2, margin), Math.max(margin, vw - bw - margin));
    let top = r.top + r.height + 8;
    if (tooltip.above && r.top - bh - 8 >= margin) top = r.top - bh - 8;
    else if (top + bh > vh - margin) top = Math.max(margin, r.top - bh - 8);
    return { left, top };
  });
</script>

<!-- A popup that is hidden (blur / tray click) must not come back with a stale tooltip on it. -->
<svelte:window
  bind:innerWidth={vw}
  bind:innerHeight={vh}
  onblur={() => tooltip.hide()}
/>
<svelte:document onvisibilitychange={() => tooltip.hide()} />

{#if tooltip.open}
  <div
    class="tip"
    role="tooltip"
    bind:clientWidth={w}
    bind:clientHeight={h}
    style="left:{pos.left}px;top:{pos.top}px;visibility:{w ? 'visible' : 'hidden'}"
  >
    {tooltip.text}
  </div>
{/if}

<style>
  .tip {
    position: fixed;
    z-index: 100;
    max-width: 250px;
    padding: 8px 11px;
    border-radius: 12px;
    background: var(--bg);
    color: var(--text);
    font-size: 11.5px;
    line-height: 1.4;
    white-space: pre-line;
    box-shadow:
      var(--raised),
      0 0 0 1px color-mix(in srgb, var(--lo) 55%, transparent);
    pointer-events: none;
  }
</style>
