<script lang="ts">
  import { smooth } from "./motion.svelte";

  let { value, color, height = 10 }: { value: number; color: string; height?: number } = $props();
  /** Glides to each new reading (instant with reduced motion). */
  const width = smooth(() => Math.max(0, Math.min(100, value)));
</script>

<div class="bar" style="height:{height}px">
  <i style="width:{width.current}%;background:{color}"></i>
</div>

<style>
  .bar {
    width: 100%;
    border-radius: 6px;
    background: var(--bg);
    box-shadow: var(--inset-sm);
    overflow: hidden;
    padding: 2px;
  }
  i {
    display: block;
    height: 100%;
    border-radius: 4px;
  }
</style>
