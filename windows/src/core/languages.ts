// Languages for the Translator tab, with Google Translate's codes. Kept free of
// imports so tests can load it without a DOM or Tauri.

export const LANGUAGES: readonly (readonly [code: string, name: string])[] = [
  ["ar", "Arabic"],
  ["be", "Belarusian"],
  ["bn", "Bengali"],
  ["bg", "Bulgarian"],
  ["ca", "Catalan"],
  ["zh-CN", "Chinese (Simplified)"],
  ["zh-TW", "Chinese (Traditional)"],
  ["hr", "Croatian"],
  ["cs", "Czech"],
  ["da", "Danish"],
  ["nl", "Dutch"],
  ["en", "English"],
  ["et", "Estonian"],
  ["fi", "Finnish"],
  ["fr", "French"],
  ["ka", "Georgian"],
  ["de", "German"],
  ["el", "Greek"],
  ["he", "Hebrew"],
  ["hi", "Hindi"],
  ["hu", "Hungarian"],
  ["id", "Indonesian"],
  ["it", "Italian"],
  ["ja", "Japanese"],
  ["kk", "Kazakh"],
  ["ko", "Korean"],
  ["lv", "Latvian"],
  ["lt", "Lithuanian"],
  ["ms", "Malay"],
  ["no", "Norwegian"],
  ["fa", "Persian"],
  ["pl", "Polish"],
  ["pt", "Portuguese"],
  ["ro", "Romanian"],
  ["ru", "Russian"],
  ["sr", "Serbian"],
  ["sk", "Slovak"],
  ["sl", "Slovenian"],
  ["es", "Spanish"],
  ["sv", "Swedish"],
  ["th", "Thai"],
  ["tr", "Turkish"],
  ["uk", "Ukrainian"],
  ["ur", "Urdu"],
  ["uz", "Uzbek"],
  ["vi", "Vietnamese"],
];

/** Older codes Google may still answer with. */
const ALIASES: Record<string, string> = { iw: "he", jw: "jv", zh: "zh-CN" };

function normalise(code: string): string {
  const exact = LANGUAGES.find(([c]) => c.toLowerCase() === code.toLowerCase());
  return exact ? exact[0] : (ALIASES[code.toLowerCase()] ?? code);
}

/** "fr" → "French"; unknown codes come back as they are. */
export function languageName(code: string): string {
  const c = normalise(code);
  return LANGUAGES.find(([k]) => k === c)?.[1] ?? code;
}

/** The language to translate into by default: the system's, or English. */
export function defaultTarget(locale: string | undefined): string {
  const tag = (locale ?? "").toLowerCase();
  if (tag.startsWith("zh")) {
    return /-(tw|hk|mo|hant)/.test(tag) ? "zh-TW" : "zh-CN";
  }
  const base = tag.split("-")[0];
  return LANGUAGES.some(([c]) => c === base) ? base : "en";
}

/**
 * The swap button. With "detect" as the source, the language Google found
 * becomes the new target. When there is nothing sensible to swap (nothing
 * detected yet, or both sides the same) the pair comes back unchanged.
 */
export function swapLanguages(
  source: string,
  target: string,
  detected: string | null,
): { source: string; target: string } {
  const from = source === "auto" ? (detected ? normalise(detected) : null) : source;
  if (!from || from === target) return { source, target };
  return { source: target, target: from };
}

/** The same text on translate.google.com, for when there is no API key. */
export function googleTranslateUrl(text: string, source: string, target: string): string {
  const q = new URLSearchParams({ sl: source, tl: target, text, op: "translate" });
  return `https://translate.google.com/?${q.toString()}`;
}
