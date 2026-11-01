<script lang="ts">
  import Tile from "../Tile.svelte";
  import Kv from "../Kv.svelte";
  import { tr, dur } from "../tr";
  import { store, type Metrics } from "../metrics.svelte";
  import { settings } from "../settings.svelte";

  let { m, adv, span }: { m: Metrics; adv: boolean; span: number } = $props();
  const hw = $derived(store.hw);
  const show = (row: string) => settings.shown("system", row);
</script>

<Tile title={tr("system")} icon="system" color="var(--net)" {span} badge={adv && settings.badgeOf("system") !== "none" ? dur(m.uptimeSecs) : undefined}>
  {#if adv}
    <div class="kv">
      {#if hw?.os && show("os")}<Kv label={tr("os")} value={hw.os} />{/if}
      {#if hw?.osBuild && show("build")}<Kv label={tr("buildNo")} value={hw.osBuild} />{/if}
      {#if hw?.board && show("board")}<Kv label={tr("board")} value={hw.board} wrap />{/if}
      {#if show("uptime")}<Kv label={tr("uptimeFull")} value={dur(m.uptimeSecs)} />{/if}
      {#if show("procs")}<Kv label={tr("procCount")} value={String(m.processCount)} />{/if}
      {#if show("threads")}<Kv label={tr("threadCount")} value={String(m.threadCount)} />{/if}
      {#if show("handles")}<Kv label={tr("handles")} value={String(m.handleCount)} />{/if}
    </div>
  {:else}
    <div class="stack c">
      <b class="big">{dur(m.uptimeSecs)}</b>
      <span class="sub">{tr("uptimeFull")}</span>
      {#if hw?.os}<span class="sub os" title={hw.os}>{hw.os}</span>{/if}
      <span class="sub">{m.processCount} {tr("procCount")}</span>
    </div>
  {/if}
</Tile>

<style>
  .c {
    align-items: center;
    text-align: center;
    gap: 3px;
  }
  .big {
    font-size: 22px;
    font-weight: 650;
  }
  .os {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
