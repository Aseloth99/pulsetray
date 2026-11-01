const SIZE = ["B", "KB", "MB", "GB", "TB"];

export function bytes(n: number, digits = 1): string {
  let v = n;
  let i = 0;
  while (v >= 1024 && i < SIZE.length - 1) {
    v /= 1024;
    i++;
  }
  return `${i === 0 ? v.toFixed(0) : v.toFixed(digits)} ${SIZE[i]}`;
}

export function rate(n: number): string {
  return `${bytes(n)}/s`;
}

export function pct(used: number, total: number): number {
  return total > 0 ? Math.min(100, (used / total) * 100) : 0;
}

export interface Units {
  d: string;
  h: string;
  m: string;
}

export function duration(secs: number, u: Units = { d: "d", h: "h", m: "m" }): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}${u.d} ${h}${u.h}`;
  if (h > 0) return `${h}${u.h} ${m}${u.m}`;
  return `${m}${u.m}`;
}

/** "5.6 / 7.2 GB": one shared unit, picked from the total, so it fits narrow tiles. */
export function pairBytes(used: number, total: number, compact = false): string {
  let i = 0;
  let t = total;
  while (t >= 1024 && i < SIZE.length - 1) {
    t /= 1024;
    i++;
  }
  const div = 1024 ** i;
  const sep = compact ? "/" : " / ";
  return `${(used / div).toFixed(1)}${sep}${(total / div).toFixed(1)} ${SIZE[i]}`;
}

/** Drive sizes with as many decimals as it takes to tell them apart: "46 GB", "4.5 GB", "1.8 TB". */
export function size(n: number): string {
  let v = n;
  let i = 0;
  while (v >= 1024 && i < SIZE.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i > 0 && v < 10 ? 1 : 0)} ${SIZE[i]}`;
}

/**
 * Used / total of a drive in the total's unit, precise enough that a nearly full drive doesn't read
 * "2 TB / 2 TB": "1.77 / 1.82 TB", "43.6 / 54.1 GB", "695 / 931 GB".
 */
export function sizePair(used: number, total: number): string {
  let i = 0;
  let t = total;
  while (t >= 1024 && i < SIZE.length - 1) {
    t /= 1024;
    i++;
  }
  const div = 1024 ** i;
  const digits = i === 0 ? 0 : t < 10 ? 2 : t < 100 ? 1 : 0;
  return `${(used / div).toFixed(digits)} / ${t.toFixed(digits)} ${SIZE[i]}`;
}

/** A drive model that already names its size ("Samsung SSD 990 PRO 2TB") doesn't need it again. */
export function namesSize(model: string): boolean {
  return /\d+(?:[.,]\d+)?\s?(?:TB|GB)\b/i.test(model);
}

export function ghz(mhz: number): string {
  return mhz >= 1000 ? `${(mhz / 1000).toFixed(2)} GHz` : `${mhz} MHz`;
}

/** Cache sizes arrive in KB. */
export function cacheSize(kb: number): string {
  return kb >= 1024 ? `${(kb / 1024).toFixed(kb % 1024 === 0 ? 0 : 1)} MB` : `${kb} KB`;
}

export function wh(mwh: number): string {
  return `${(mwh / 1000).toFixed(1)} Wh`;
}

export function watts(mw: number): string {
  return `${(Math.abs(mw) / 1000).toFixed(1)} W`;
}

/** Drop vendor noise so long CPU/GPU names fit: "AMD Ryzen 5 7520U with Radeon Graphics" -> "Ryzen 5 7520U". */
export function shortName(name: string): string {
  return name
    .replace(/\(R\)|\(TM\)|\(tm\)|\(r\)/g, "")
    .replace(/\s+with\s+Radeon.*$/i, "")
    .replace(/\s+\d+-Core Processor$/i, "")
    .replace(/\b(Processor|CPU)\b/gi, "")
    .replace(/^(AMD|Intel|NVIDIA)\s+/i, "")
    .replace(/\s+/g, " ")
    .trim();
}

/** Wall-clock time of a unix timestamp: "14:32" today, "12 Oct 14:32" on another day. */
export function clock(unixSecs: number, lang: string): string {
  const d = new Date(unixSecs * 1000);
  const loc = lang === "tr" ? "tr-TR" : "en-US";
  const time = d.toLocaleTimeString(loc, { hour: "2-digit", minute: "2-digit", hour12: false });
  if (d.toDateString() === new Date().toDateString()) return time;
  return `${d.toLocaleDateString(loc, { day: "numeric", month: "short" })} ${time}`;
}

/**
 * The Turkish possessive ending after a number as it is read out: %2'si (ikisi), %5'i (beşi), %10'u (onu),
 * %40'ı (kırkı), %100'ü (yüzü). A fixed "'i" made "Ömrün %2'i kullanıldı".
 */
export function trPossessive(n: number | string): string {
  const digits = String(n).replace(/[^0-9]/g, "");
  if (!digits || /^0+$/.test(digits)) return "ı"; // sıfırı
  const ONES = ["", "i", "si", "ü", "ü", "i", "sı", "si", "i", "u"]; // bir iki üç dört beş altı yedi sekiz dokuz
  const TENS = ["", "u", "si", "u", "ı", "si", "ı", "i", "i", "ı"]; // on yirmi otuz kırk elli altmış yetmiş seksen doksan
  const last = Number(digits[digits.length - 1]);
  if (last) return ONES[last];
  const tens = digits.length > 1 ? Number(digits[digits.length - 2]) : 0;
  if (tens) return TENS[tens];
  const zeros = digits.length - digits.replace(/0+$/, "").length;
  // 100, 1100 ... yüzü; 1000, 20 000, 300 000 ... bini; milyonu; milyarı
  return zeros === 2 ? "ü" : zeros <= 5 ? "i" : zeros <= 8 ? "u" : "ı";
}

/** Chemistry codes from the battery report (4 letters from the firmware) in words: "Li-I" / "LION" -> "Li-ion". */
export function chemistryName(code: string): string {
  const k = code.toLowerCase().replace(/[^a-z]/g, "");
  if (["lion", "lii", "liion", "li"].includes(k)) return "Li-ion";
  if (["lip", "lipo", "lipoly", "lipolymer", "lpol"].includes(k)) return "Li-polymer";
  if (k === "nimh") return "NiMH";
  if (k === "nicd") return "NiCd";
  if (k === "nizn") return "NiZn";
  if (k === "pbac") return "Pb-acid";
  return code.trim();
}

/** mm:ss for a countdown. */
export function mmss(secs: number): string {
  return `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`;
}

// Chart readouts (values as the store keeps them).
export const percent = (v: number): string => `${v.toFixed(0)}%`;
export const celsius = (v: number): string => `${v.toFixed(0)} °C`;
export const wattsNum = (w: number): string => `${w.toFixed(w < 10 ? 1 : 0)} W`;
