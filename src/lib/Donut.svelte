<script lang="ts">
  let {
    value,
    color,
    size = 84,
    label,
  }: { value: number; color: string; size?: number; label?: string } = $props();

  const stroke = 9;
  const r = $derived((size - stroke) / 2 - 6);
  const circ = $derived(2 * Math.PI * r);
  const v = $derived(Math.max(0, Math.min(100, value)));
  const fs = $derived(Math.max(11, Math.round(size * 0.2)));
</script>

<div class="donut" style="width:{size}px;height:{size}px">
  <svg width={size} height={size} viewBox="0 0 {size} {size}">
    <circle class="track" cx={size / 2} cy={size / 2} r={r} stroke-width={stroke} />
    <circle
      class="arc"
      cx={size / 2}
      cy={size / 2}
      r={r}
      stroke-width={stroke}
      stroke={color}
      stroke-dasharray="{(circ * v) / 100} {circ}"
      transform="rotate(-90 {size / 2} {size / 2})"
    />
  </svg>
  <div class="center">
    <b style="font-size:{fs}px">{Math.round(v)}<small>%</small></b>
    {#if label}<span>{label}</span>{/if}
  </div>
</div>

<style>
  .donut {
    position: relative;
    border-radius: 50%;
    background: var(--bg);
    box-shadow: var(--inset);
    flex: none;
  }
  svg {
    position: absolute;
    inset: 0;
    fill: none;
  }
  .track {
    stroke: transparent;
  }
  .arc {
    stroke-linecap: round;
  }
  .center {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    text-align: center;
    line-height: 1.1;
  }
  b {
    font-weight: 650;
  }
  small {
    font-size: 10px;
    font-weight: 500;
    color: var(--muted);
    margin-left: 1px;
  }
  span {
    font-size: 9px;
    color: var(--muted);
  }
</style>
