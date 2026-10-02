// Translator, free half: Google Translate's public web endpoint, the one browser
// extensions and command-line tools use. No key, no billing account. It is not
// an official API, so Google may slow it down or change it.
//
// The island calls it itself rather than through Rust: the endpoint answers
// browsers, while the app's own HTTP client gets Google's anti-bot page (429).
// Kept free of imports so tests can load it without a DOM or Tauri.

export const FREE_ENDPOINT = "https://translate.googleapis.com/translate_a/single";

const TIMEOUT_MS = 20_000;

export interface FreeTranslation {
  text: string;
  /** Language Google recognised when the source was "auto". */
  detectedSource: string | null;
}

/** Languages go in the URL; the text goes in the body, so long texts fit. */
export function freeRequest(text: string, source: string, target: string): { url: string; body: string } {
  const query = new URLSearchParams({ client: "gtx", sl: source, tl: target, dt: "t" });
  return { url: `${FREE_ENDPOINT}?${query.toString()}`, body: new URLSearchParams({ q: text }).toString() };
}

/**
 * The answer is a list with one entry per sentence (translation first,
 * original second), then the detected language.
 */
export function parseFreeResponse(data: unknown): FreeTranslation {
  const unexpected = "Google Translate sent something unexpected. Try again in a moment.";
  if (!Array.isArray(data) || !Array.isArray(data[0])) throw new Error(unexpected);
  const text = (data[0] as unknown[])
    .map((sentence) => (Array.isArray(sentence) && typeof sentence[0] === "string" ? sentence[0] : ""))
    .join("");
  if (!text) throw new Error(unexpected);
  return { text, detectedSource: typeof data[2] === "string" ? data[2] : null };
}

export async function translateFree(text: string, source: string, target: string): Promise<FreeTranslation> {
  const { url, body } = freeRequest(text, source, target);
  let response: Response;
  try {
    response = await fetch(url, {
      method: "POST",
      // A form body keeps this a simple cross-origin request: no preflight.
      headers: { "Content-Type": "application/x-www-form-urlencoded;charset=UTF-8" },
      body,
      credentials: "omit",
      referrerPolicy: "no-referrer",
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
  } catch (err) {
    throw new Error(
      (err as Error)?.name === "TimeoutError"
        ? "Google Translate did not answer in time."
        : "Can't reach Google Translate. Check the connection.",
    );
  }
  if (response.status === 429) {
    throw new Error("Google Translate's free service is busy. Wait a minute, or add an API key in Settings → Translator.");
  }
  if (!response.ok) {
    throw new Error(
      `Google Translate's free service refused (${response.status}). Try again later, or add an API key in Settings → Translator.`,
    );
  }
  return parseFreeResponse(await response.json().catch(() => null));
}
