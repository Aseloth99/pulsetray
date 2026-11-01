class TooltipState {
  text = $state("");
  open = $state(false);
  rect = $state({ left: 0, top: 0, width: 0, height: 0 });
  /** Open above the element when there is room (chart readouts: the text below the chart stays visible). */
  above = $state(false);

  show(text: string, r: DOMRect, above = false) {
    this.text = text;
    this.above = above;
    this.rect = { left: r.left, top: r.top, width: r.width, height: r.height };
    this.open = true;
  }

  hide() {
    this.open = false;
  }
}

export const tooltip = new TooltipState();

/**
 * Focus shows a tooltip only for keyboard users: when the popup opens, the browser puts focus on the
 * first button, and that must not pop a tooltip up on a button nobody is pointing at.
 */
let keyboard = false;
if (typeof window !== "undefined") {
  window.addEventListener("keydown", (e) => (keyboard = e.key === "Tab" || keyboard), true);
  window.addEventListener("mousedown", () => (keyboard = false), true);
  window.addEventListener("mousemove", () => (keyboard = false), true);
  window.addEventListener("blur", () => (keyboard = false));
}

/**
 * Svelte action: `use:tip={"explanation"}` shows a styled tooltip after a short hover.
 * (Rendered by <Tooltip/> at the page root, so it is never clipped by a scrolling tile.)
 */
export function tip(node: HTMLElement, text: string) {
  let current = text;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const show = () => {
    if (!current) return;
    clearTimeout(timer);
    timer = setTimeout(() => tooltip.show(current, node.getBoundingClientRect()), 350);
  };
  const onFocus = () => {
    if (keyboard) show();
  };
  const hide = () => {
    clearTimeout(timer);
    tooltip.hide();
  };

  node.addEventListener("mouseenter", show);
  node.addEventListener("mouseleave", hide);
  node.addEventListener("mousedown", hide);
  node.addEventListener("focusin", onFocus);
  node.addEventListener("focusout", hide);

  return {
    update(next: string) {
      current = next;
      if (!next) hide();
    },
    destroy() {
      hide();
      node.removeEventListener("mouseenter", show);
      node.removeEventListener("mouseleave", hide);
      node.removeEventListener("mousedown", hide);
      node.removeEventListener("focusin", onFocus);
      node.removeEventListener("focusout", hide);
    },
  };
}
