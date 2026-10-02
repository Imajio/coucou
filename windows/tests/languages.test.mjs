// Translator tab helpers: language names, defaults, swapping, browser fallback.
//
//   npm test

import { test } from "node:test";
import assert from "node:assert/strict";

const { languageName, defaultTarget, swapLanguages, googleTranslateUrl, LANGUAGES } = await import(
  "../src/core/languages.ts"
);

test("codes read as language names, old codes included", () => {
  assert.equal(languageName("fr"), "French");
  assert.equal(languageName("zh-cn"), "Chinese (Simplified)");
  assert.equal(languageName("iw"), "Hebrew");
  assert.equal(languageName("xx"), "xx");
});

test("the default target follows the system language", () => {
  assert.equal(defaultTarget("ru-RU"), "ru");
  assert.equal(defaultTarget("pt-BR"), "pt");
  assert.equal(defaultTarget("zh-TW"), "zh-TW");
  assert.equal(defaultTarget("zh-Hans-CN"), "zh-CN");
  assert.equal(defaultTarget("tlh"), "en");
  assert.equal(defaultTarget(undefined), "en");
});

test("swapping turns the target into the source", () => {
  assert.deepEqual(swapLanguages("en", "fr", null), { source: "fr", target: "en" });
  assert.deepEqual(swapLanguages("auto", "fr", "de"), { source: "fr", target: "de" });
  assert.deepEqual(swapLanguages("auto", "ru", "iw"), { source: "ru", target: "he" });
});

test("swapping with nothing to swap changes nothing", () => {
  assert.deepEqual(swapLanguages("auto", "en", null), { source: "auto", target: "en" });
  assert.deepEqual(swapLanguages("auto", "fr", "fr"), { source: "auto", target: "fr" });
});

test("the browser fallback carries the text and both languages", () => {
  const url = new URL(googleTranslateUrl("Hello & bye?", "auto", "fr"));
  assert.equal(url.origin, "https://translate.google.com");
  assert.equal(url.searchParams.get("sl"), "auto");
  assert.equal(url.searchParams.get("tl"), "fr");
  assert.equal(url.searchParams.get("text"), "Hello & bye?");
});

test("every language has a unique code", () => {
  const codes = LANGUAGES.map(([c]) => c);
  assert.equal(new Set(codes).size, codes.length);
});
