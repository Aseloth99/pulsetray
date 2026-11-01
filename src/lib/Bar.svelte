<script lang="ts">
  let { value, color, height = 10 }: { value: number; color: string; height?: number } = $props();
</script>

<div class="bar" style="height:{height}px">
  <i style="width:{Math.max(0, Math.min(100, value))}%;background:{color}"></i>
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
