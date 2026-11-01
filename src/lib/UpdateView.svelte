<script lang="ts">
  import { update } from "./update.svelte";
  import { tr } from "./tr";
  import { bytes, rate } from "./format";
  import { notesToText } from "./notes";
  import { settings } from "./settings.svelte";
  import Icon from "./Icon.svelte";

  /**
   * The update screen (opened by the pill in the popup, the tray menu's "Update · vX" and Settings): versions, download
   * size, release notes, then "Later" / "Update". "Update" downloads with a progress bar and a Cancel button, then installs
   * quietly and the app restarts by itself. A failure stays on this screen with "Try again"; the old version keeps running.
   */
  let { current, onLater }: { current: string; onLater: () => void } = $props();

  const offer = $derived(update.offer);
  const st = $derived(update.phase.state);
  /** The notes as plain text (the server's HTML never reaches the page as markup). */
  const notes = $derived(offer ? notesToText(offer.notes) : "");

  /** "Update" was pressed and the backend has not answered yet (the screen reacts at once). */
  let pressed = $state(false);
  let pressedTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    st; // any change of state means the backend heard us
    pressed = false;
    clearTimeout(pressedTimer);
  });
  async function press() {
    if (pressed) return;
    pressed = true;
    clearTimeout(pressedTimer);
    pressedTimer = setTimeout(() => (pressed = false), 5000);
    if (!(await update.go())) pressed = false;
  }

  const downloading = $derived(st === "downloading" || (pressed && st !== "installing"));
  const installing = $derived(st === "installing");
  const pct = $derived(
    st === "downloading" && update.total > 0 ? Math.min(100, Math.floor((update.done / update.total) * 100)) : null,
  );
  const pctText = (p: number) => (settings.lang === "tr" ? `%${p}` : `${p}%`);
  const progress = $derived.by(() => {
    if (st !== "downloading") return "";
    const parts: string[] = [];
    if (pct !== null) parts.push(pctText(pct));
    parts.push(update.total > 0 ? `${bytes(update.done)} / ${bytes(update.total)}` : bytes(update.done));
    if (update.speed > 0) parts.push(rate(update.speed));
    return parts.join(" · ");
  });

  /** The backend reports a code ("checksum", "http: 503", "network: ..."); the sentence is made here, in the chosen language. */
  function errText(raw: string): string {
    const at = raw.indexOf(":");
    const code = (at < 0 ? raw : raw.slice(0, at)).trim();
    const detail = at < 0 ? "" : raw.slice(at + 1).trim();
    switch (code) {
      case "checksum":
        return tr("updateErrChecksum");
      case "size":
        return tr("updateErrSize");
      case "too_large":
        return tr("updateErrTooLarge");
      case "package":
        return tr("updateErrPackage");
      case "arch":
        return tr("updateErrArch");
      case "not_writable":
        return tr("updateErrWrite");
      case "network":
        return tr("updateErrNetwork");
      case "http":
        return tr("updateErrHttp").replace("{n}", detail);
      case "url":
        return tr("updateErrUrl");
      case "no_offer":
        return tr("updateErrNoOffer");
      default:
        return tr("updateErrOther").replace("{e}", detail || raw);
    }
  }
</script>

<div class="view">
  {#if !offer}
    <section class="card">
      <p class="state">{tr("upToDate")}</p>
    </section>
    <div class="actions">
      <button class="btn" onclick={onLater}>{tr("back")}</button>
    </div>
  {:else}
    <section class="card">
      <div class="ver">
        <Icon name="update" size={22} />
        <span class="vtext">{tr("updateFrom").replace("{from}", current).replace("{to}", offer.version)}</span>
      </div>
      {#if offer.size}
        <p class="sub">{tr("updateSize").replace("{size}", bytes(offer.size))}</p>
      {/if}
    </section>

    <section class="card notescard">
      <h3>{tr("updateNotes")}</h3>
      <!-- Plain text only: never {@html} (the notes come from a server). Focusable so that the keyboard can scroll it. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div class="notes" tabindex="0" role="region" aria-label={tr("updateNotes")}>
        {#if notes}{notes}{:else}<span class="sub">{tr("updateNoNotes")}</span>{/if}
      </div>
    </section>

    {#if downloading}
      <div class="status" aria-live="polite">
        <div class="line">
          <span>{tr("updateDownloading")}</span>
          <span class="num">{progress}</span>
        </div>
        <div
          class="bar"
          class:indet={pct === null}
          role="progressbar"
          aria-label={tr("updateDownloading")}
          aria-valuemin="0"
          aria-valuemax="100"
          aria-valuenow={pct ?? undefined}
        >
          <i style:width={pct === null ? undefined : `${pct}%`}></i>
        </div>
      </div>
    {:else if installing}
      <div class="status inst" aria-live="polite">
        <span class="spin" aria-hidden="true"></span>
        <span>{tr("updateInstalling")}</span>
      </div>
    {:else if st === "failed"}
      <div class="err" role="alert">
        {tr("updateFailed").replace("{error}", errText(update.phase.error ?? ""))}
      </div>
    {:else if st === "ready"}
      <p class="ok">{tr("updatePending").replace("{v}", offer.version)}</p>
    {/if}

    {#if !installing}
      <div class="actions">
        {#if downloading}
          <button class="btn" disabled={st !== "downloading"} onclick={() => update.cancel()}>{tr("updateCancel")}</button>
        {:else}
          <button class="btn" onclick={onLater}>{tr("updateLater")}</button>
          <button class="btn primary" disabled={pressed} onclick={press}>
            {st === "failed" ? tr("updateRetry") : tr("updateNow")}
          </button>
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  .card {
    background: var(--bg);
    border-radius: 20px;
    box-shadow: var(--raised);
    padding: 13px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .card h3 {
    font-size: 12px;
    font-weight: 650;
    color: var(--muted);
  }
  .ver {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--net-text);
  }
  .ver :global(svg) {
    flex: none;
  }
  .vtext {
    min-width: 0;
    font-size: 17px;
    font-weight: 700;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .state {
    color: var(--muted);
    overflow-wrap: anywhere;
  }
  .notes {
    max-height: 220px;
    overflow-y: auto;
    padding: 10px 12px;
    border-radius: 14px;
    box-shadow: var(--inset-sm);
    font-size: 12.5px;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
    outline: none;
  }
  .notes:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--net) 60%, transparent);
    outline-offset: 1px;
  }
  .notes::-webkit-scrollbar {
    width: 6px;
  }
  .notes::-webkit-scrollbar-thumb {
    border-radius: 3px;
    background: color-mix(in srgb, var(--muted) 45%, transparent);
  }
  .status {
    display: flex;
    flex-direction: column;
    gap: 7px;
    font-size: 12px;
  }
  .status.inst {
    flex-direction: row;
    align-items: center;
    gap: 9px;
    font-weight: 600;
  }
  .line {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: 2px 10px;
    font-weight: 600;
  }
  .num {
    color: var(--muted);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }
  .bar {
    height: 10px;
    border-radius: 6px;
    box-shadow: var(--inset-sm);
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    border-radius: 6px;
    background: var(--net);
    transition: width 0.25s linear;
  }
  .bar.indet i {
    width: 35%;
    animation: slide 1.1s ease-in-out infinite alternate;
  }
  @keyframes slide {
    from {
      transform: translateX(0);
    }
    to {
      transform: translateX(185%);
    }
  }
  .spin {
    flex: none;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    border: 2px solid var(--net);
    border-right-color: transparent;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .err {
    padding: 9px 12px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--danger) 13%, transparent);
    color: var(--bad-text);
    font-size: 12px;
    line-height: 1.45;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .ok {
    font-size: 12px;
    font-weight: 600;
    color: var(--ok-text);
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }
  .btn {
    flex: 1 1 auto;
    padding: 10px 14px;
    border-radius: 13px;
    box-shadow: var(--raised-sm);
    font-weight: 600;
    text-align: center;
  }
  .btn.primary {
    color: var(--net-text);
  }
  .btn:active:not(:disabled) {
    box-shadow: var(--inset-sm);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
