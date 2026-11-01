/**
 * Release notes come from the server as HTML or plain text. They are NEVER inserted as markup (no `{@html}`): this turns
 * them into plain text with line breaks. The parsing happens in an inert document (`DOMParser` runs no scripts and loads
 * no images), and only text nodes are read out of it.
 */
const MAX = 20000;

/** Elements that start and end a line (a blank line after the ones that read as paragraphs). */
const BLOCK = new Set([
  "ADDRESS", "ARTICLE", "ASIDE", "BLOCKQUOTE", "DD", "DETAILS", "DIV", "DL", "DT", "FIELDSET", "FIGURE", "FOOTER",
  "FORM", "H1", "H2", "H3", "H4", "H5", "H6", "HEADER", "HR", "LI", "MAIN", "NAV", "OL", "P", "PRE", "SECTION",
  "TABLE", "TR", "UL",
]);
const PARAGRAPH = new Set(["BLOCKQUOTE", "H1", "H2", "H3", "H4", "H5", "H6", "OL", "P", "PRE", "TABLE", "UL"]);
/** Content that is not text for a reader. */
const SKIP = new Set(["SCRIPT", "STYLE", "HEAD", "TEMPLATE", "NOSCRIPT", "IFRAME", "OBJECT", "EMBED", "SVG", "CANVAS", "TITLE"]);

const LOOKS_LIKE_HTML = /<\/?[a-z][^>]*>|&(#\d+|#x[0-9a-f]+|[a-z]{2,8});/i;
const HAS_BLOCKS = /<(br|p|li|div|h[1-6]|ul|ol|tr|table|pre|blockquote|section|article)\b/i;

export function notesToText(raw: string): string {
  const src = String(raw ?? "").replace(/\r\n?/g, "\n");
  if (!LOOKS_LIKE_HTML.test(src)) return src.trim().slice(0, MAX);

  const doc = new DOMParser().parseFromString(src, "text/html");
  // Tags without any block element in the text ("<b>New</b>\nsecond line"): the text's own line breaks are the layout.
  const keepNewlines = !HAS_BLOCKS.test(src);
  let out = "";
  const newline = (blank = false) => {
    if (!out) return;
    if (!out.endsWith("\n")) out += "\n";
    if (blank && !out.endsWith("\n\n")) out += "\n";
  };

  const walk = (node: Node, pre: boolean) => {
    if (node.nodeType === Node.TEXT_NODE) {
      const t = node.nodeValue ?? "";
      out += pre ? t : keepNewlines ? t.replace(/[ \t\f\v]+/g, " ") : t.replace(/\s+/g, " ");
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const el = node as Element;
    const tag = el.tagName.toUpperCase();
    if (SKIP.has(tag)) return;
    if (tag === "BR") {
      out += "\n";
      return;
    }
    const block = BLOCK.has(tag);
    if (block) newline();
    if (tag === "LI") {
      const parent = el.parentElement?.tagName.toUpperCase();
      out += parent === "OL" ? `${Array.prototype.indexOf.call(el.parentElement!.children, el) + 1}. ` : "• ";
    }
    for (const child of Array.from(el.childNodes)) walk(child, pre || tag === "PRE");
    if (block) newline(PARAGRAPH.has(tag));
  };
  walk(doc.body, false);

  return out
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n[ \t]+/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim()
    .slice(0, MAX);
}
