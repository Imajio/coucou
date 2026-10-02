// Translator tab: Google Translate from the island. Free by default (Google's
// public web service); the official API when a key is saved in Settings. When a
// translation fails, the same text can still be opened on translate.google.com.

import { h, svg } from "./dom";
import { ICONS } from "./icons";
import { Bridge } from "../core/bridge";
import { translateFree } from "../core/translate";
import { Sound } from "../core/sound";
import {
  LANGUAGES, defaultTarget, googleTranslateUrl, languageName, swapLanguages,
} from "../core/languages";
import type { ViewActions, ViewHost } from "./views";

const MAX_CHARS = 5000;
const PREF_SOURCE = "coucou.translate.source";
const PREF_TARGET = "coucou.translate.target";
const API_KEY = "google-translate-api-key";

const known = (code: string | null) => code != null && (code === "auto" || LANGUAGES.some(([c]) => c === code));

/** Remembered per machine; storage can be unavailable, which only loses the choice. */
function loadPref(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function savePref(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Nothing to do: the choice just won't survive a restart.
  }
}

function languageSelect(withDetect: boolean): HTMLSelectElement {
  const sel = h("select", { class: "tr-select" }) as HTMLSelectElement;
  if (withDetect) sel.append(h("option", { value: "auto", text: "Detect language" }));
  for (const [code, name] of LANGUAGES) sel.append(h("option", { value: code, text: name }));
  return sel;
}

export function buildTranslate(actions: ViewActions): ViewHost {
  let source = known(loadPref(PREF_SOURCE)) ? loadPref(PREF_SOURCE)! : "auto";
  let target = known(loadPref(PREF_TARGET)) && loadPref(PREF_TARGET) !== "auto"
    ? loadPref(PREF_TARGET)!
    : defaultTarget(navigator.language);
  let detected: string | null = null;
  let busy = false;
  /** The last translation failed: offer the browser instead. */
  let failed = false;
  let lastKey = "";

  const sourceSel = languageSelect(true);
  const targetSel = languageSelect(false);
  const swap = h("button", { class: "tr-swap", title: "Swap languages" }, svg(ICONS.swap, 12));
  const web = h("button", { class: "link-btn tr-web", title: "Open this text on translate.google.com" },
    h("span", { text: "Google Translate" }), svg(ICONS.arrowUpRight, 9));

  const input = h("textarea", {
    class: "tr-input",
    placeholder: "Type or paste text…",
    spellcheck: "false",
    maxlength: String(MAX_CHARS),
  }) as HTMLTextAreaElement;
  const output = h("div", { class: "tr-output", "data-placeholder": "Translation" });
  const status = h("span", { class: "tr-status" });

  const copy = h("button", { class: "btn secondary tr-small" }, svg(ICONS.copy, 11), h("span", { text: "Copy" }));
  const go = h("button", { class: "btn primary tr-small" }, h("span", { text: "Translate" }), h("span", { class: "kbd", text: "↵" }));
  const openWeb = h("button", { class: "btn secondary tr-small" }, h("span", { text: "Open in browser" }), svg(ICONS.arrowUpRight, 9));

  const body = h(
    "div",
    { class: "tr-body" },
    h("div", { class: "tr-langs" }, sourceSel, swap, targetSel, h("div", { class: "grow" }), web),
    h("div", { class: "tr-boxes" }, input, output),
    h("div", { class: "tr-footer" }, status, h("div", { class: "grow" }), openWeb, copy, go),
  );
  const cardEl = h("div", { class: "card wash" }, body);
  cardEl.style.setProperty("--wash", "rgba(34,211,238,0.34)");
  const el = h("div", { class: "view" }, cardEl);

  function setStatus(text: string, kind: "" | "err" | "ok" = "") {
    status.textContent = text;
    status.title = text; // long messages are cut off; the tooltip has all of it
    status.className = kind ? `tr-status ${kind}` : "tr-status";
  }

  function render() {
    sourceSel.value = source;
    targetSel.value = target;
    const swapped = swapLanguages(source, target, detected);
    swap.disabled = swapped.source === source && swapped.target === target;
    openWeb.style.display = failed ? "" : "none";
    copy.disabled = !output.textContent;
    go.disabled = busy;
    output.classList.toggle("busy", busy);
  }

  function openInBrowser() {
    actions.openUrl(googleTranslateUrl(input.value.trim(), source, target));
  }

  async function run(force = false) {
    const text = input.value.trim();
    if (!text) {
      output.textContent = "";
      detected = null;
      lastKey = "";
      setStatus("");
      render();
      return;
    }
    const key = `${source}|${target}|${text}`;
    if (busy || (key === lastKey && !force)) return;
    busy = true;
    setStatus("Translating…");
    render();
    try {
      // A saved key means the official API (through Rust, so the key never
      // reaches this page); otherwise Google Translate's free web service.
      const hasKey = (await Bridge.secretPresent(API_KEY)) ?? false;
      const result = hasKey
        ? await Bridge.translate(text, source, target)
        : await translateFree(text, source, target);
      output.textContent = result.text;
      detected = result.detectedSource;
      lastKey = key;
      failed = false;
      setStatus(source === "auto" && detected ? `Detected ${languageName(detected)}` : "");
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      lastKey = "";
      failed = true;
      setStatus(message, "err");
      Sound.play("error");
    } finally {
      busy = false;
      render();
    }
  }

  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void run(true);
    }
  });
  input.addEventListener("input", () => {
    if (!input.value.trim()) void run();
  });

  sourceSel.addEventListener("change", () => {
    source = sourceSel.value;
    savePref(PREF_SOURCE, source);
    detected = null;
    void run();
    render();
  });
  targetSel.addEventListener("change", () => {
    target = targetSel.value;
    savePref(PREF_TARGET, target);
    void run();
    render();
  });
  swap.addEventListener("click", () => {
    const next = swapLanguages(source, target, detected);
    source = next.source;
    target = next.target;
    savePref(PREF_SOURCE, source);
    savePref(PREF_TARGET, target);
    // Like Google Translate: the translation becomes the text to translate back.
    if (output.textContent) input.value = output.textContent;
    output.textContent = "";
    detected = null;
    void run();
    render();
  });
  copy.addEventListener("click", () => {
    const text = output.textContent ?? "";
    if (!text) return;
    void navigator.clipboard.writeText(text).then(
      () => setStatus("Copied", "ok"),
      () => setStatus("Couldn't copy", "err"),
    );
  });
  go.addEventListener("click", () => void run(true));
  web.addEventListener("click", openInBrowser);
  openWeb.addEventListener("click", openInBrowser);

  render();

  return {
    el,
    sync() {},
    focus() {
      input.focus();
    },
  };
}
