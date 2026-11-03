<script module lang="ts">
  export type Tone = "neutral" | "ok" | "warn" | "bad";
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import OptionMenu from "./OptionMenu.svelte";
  import type { QuickPick } from "./tiledetail";
  import { tip } from "./tipstate.svelte";

  let {
    title,
    icon,
    color,
    span = 1,
    badge,
    badgeLabel,
    badgeTone = "neutral",
    badgeIcon,
    badgeTip,
    badgePick,
    actions,
    children,
  }: {
    title: string;
    icon: string;
    color: string;
    span?: number;
    /** Small pill at the right of the header (used in advanced mode, where there is room). */
    badge?: string;
    /** What the badge number is, in front of it (muted): a bare "52.5 / 60.8 GB" read as RAM. */
    badgeLabel?: string;
    badgeTone?: Tone;
    badgeIcon?: string;
    badgeTip?: string;
    /** Makes the badge a button that lists what it can show instead (same choice as in Settings). */
    badgePick?: QuickPick;
    /** Extra header controls (buttons) to the right of the badge. */
    actions?: Snippet;
    children: Snippet;
  } = $props();
</script>

<section style={span > 1 ? `grid-column: span ${span}` : undefined}>
  <header>
    <span class="ico" style="color:{color}"><Icon name={icon} size={15} /></span>
    <h2 title={title}>{title}</h2>
    {#if badge}
      {#snippet content()}
        {#if badgeIcon}<Icon name={badgeIcon} size={11} />{/if}{#if badgeLabel}<span class="bl">{badgeLabel}</span>{/if}{badge}
      {/snippet}
      {#if badgePick}
        <OptionMenu pick={badgePick} align="end">
          {#snippet trigger({ toggle, open })}
            <button
              type="button"
              class="badge pick {badgeTone}"
              aria-haspopup="menu"
              aria-expanded={open}
              use:tip={open ? "" : [badgeTip, badgePick.tip].filter(Boolean).join("\n")}
              onclick={toggle}
            >
              {@render content()}<Icon name="down" size={10} />
            </button>
          {/snippet}
        </OptionMenu>
      {:else}
        <span class="badge {badgeTone}" use:tip={badgeTip ?? ""}>{@render content()}</span>
      {/if}
    {/if}
    {@render actions?.()}
  </header>
  {@render children()}
</section>

<style>
  section {
    position: relative;
    background: var(--bg);
    border-radius: 20px;
    box-shadow: var(--raised);
    padding: 12px 13px 13px;
    display: flex;
    flex-direction: column;
    gap: 9px;
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
  }
  .ico {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 8px;
    box-shadow: var(--raised-sm);
    flex: none;
  }
  h2 {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
    letter-spacing: 0.02em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 9px;
    border-radius: 999px;
    box-shadow: var(--inset-sm);
    font-size: 10.5px;
    font-weight: 600;
    color: var(--muted);
    white-space: nowrap;
  }
  button.badge {
    border: 0;
    background: transparent;
    font-family: inherit;
    cursor: pointer;
  }
  button.badge:hover,
  button.badge[aria-expanded="true"] {
    box-shadow: var(--inset-sm), 0 0 0 1px color-mix(in srgb, var(--muted) 45%, transparent);
  }
  .badge.pick {
    padding-right: 6px;
  }
  .badge.pick :global(svg:last-child) {
    opacity: 0.65;
  }
  .badge .bl {
    color: var(--muted);
    font-weight: 500;
  }
  .badge.ok {
    color: var(--ok-text);
  }
  .badge.warn {
    color: var(--warn-text);
  }
  .badge.bad {
    color: var(--bad-text);
  }
</style>
