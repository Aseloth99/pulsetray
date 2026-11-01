<script lang="ts">
  let {
    label,
    value,
    title,
    wrap = false,
  }: { label: string; value: string; title?: string; wrap?: boolean } = $props();
</script>

<div class="kv-row" title={title ?? value}>
  <span class="k">{label}</span>
  <span class="v" class:wrap>{value}</span>
</div>

<style>
  .kv-row {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    font-size: 11.5px;
    min-width: 0;
  }
  .k {
    color: var(--muted);
    flex: none;
  }
  .v {
    font-weight: 600;
    text-align: right;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .v.wrap {
    white-space: normal;
    overflow: visible;
    overflow-wrap: anywhere;
  }
</style>
