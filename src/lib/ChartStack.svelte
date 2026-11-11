<script module lang="ts">
  /** How one chart of a tile is drawn; the tile maps every chart id of its `DETAIL` entry to one of these. */
  export interface ChartDef {
    /** Key into `store.series`. */
    series: string;
    fmt: (v: number) => string;
    max?: number;
    bytes?: boolean;
    floor?: number;
    label: string;
    color: string;
  }
</script>

<script lang="ts">
  import Spark from "./Spark.svelte";
  import OptionMenu from "./OptionMenu.svelte";
  import Icon from "./Icon.svelte";
  import { store } from "./metrics.svelte";
  import { settings, type TileId } from "./settings.svelte";
  import { caps } from "./caps.svelte";
  import { tr } from "./tr";

  let {
    tile,
    defs,
    height = 64,
    who,
    has,
  }: {
    tile: TileId;
    defs: Record<string, ChartDef>;
    height?: number;
    /** Charts of a time axis can name the apps behind a peak; a chart of one part of the tile (one disk) can't. */
    who?: "cpu" | "mem" | "disk" | "gpu";
    /** Charts this view can't draw (the GPU shown has no power sensor) are neither shown nor offered. */
    has?: (id: string) => boolean;
  } = $props();

  const ids = $derived(caps.charts(tile, has).filter((id) => defs[id]));
  const add = $derived(caps.chartAdd(tile, has));
</script>

{#each ids as id (id)}
  {@const d = defs[id]}
  <Spark
    data={store.series[d.series] ?? []}
    times={store.times}
    windowSecs={settings.historySecs}
    color={d.color}
    max={d.max}
    fmt={d.fmt}
    bytes={d.bytes}
    floor={d.floor}
    label={d.label}
    pick={caps.chartPick(tile, id, has)}
    {height}
    {who}
    detail
  />
{/each}
{#if add}
  <OptionMenu pick={add} align="start">
    {#snippet trigger({ toggle, open })}
      <button type="button" class="add" aria-haspopup="menu" aria-expanded={open} title={add.tip} onclick={toggle}>
        <Icon name="plus" size={10} />{tr("addChart")}
      </button>
    {/snippet}
  </OptionMenu>
{/if}

<style>
  .add {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 0;
    background: transparent;
    font: inherit;
    font-size: 10px;
    color: var(--muted);
    padding: 3px 8px 3px 6px;
    border-radius: 8px;
    cursor: pointer;
    opacity: 0.75;
  }
  .add:hover,
  .add:focus-visible,
  .add[aria-expanded="true"] {
    opacity: 1;
    color: var(--text);
    outline: none;
    box-shadow: var(--inset-sm);
  }
</style>
