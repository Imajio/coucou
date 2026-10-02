// Translator, free half: the request to Google Translate's web endpoint and
// how its answer is read.
//
//   npm test

import { test } from "node:test";
import assert from "node:assert/strict";

const { freeRequest, parseFreeResponse } = await import("../src/core/translate.ts");

test("the request carries the languages in the URL and the text in the body", () => {
  const { url, body } = freeRequest("Привет & пока", "auto", "en");
  const u = new URL(url);
  assert.equal(u.origin + u.pathname, "https://translate.googleapis.com/translate_a/single");
  assert.equal(u.searchParams.get("client"), "gtx");
  assert.equal(u.searchParams.get("sl"), "auto");
  assert.equal(u.searchParams.get("tl"), "en");
  assert.equal(u.searchParams.get("dt"), "t");
  assert.equal(u.searchParams.get("q"), null);
  assert.equal(new URLSearchParams(body).get("q"), "Привет & пока");
});

test("the answer is joined back into one text with the detected language", () => {
  // Shape of a real answer: one entry per sentence, then the detected language.
  const data = JSON.parse(
    '[[["Bonjour le monde. ","Hello world. ",null,null,10],["Comment vas-tu?\\n","How are you?\\n",null,null,10,null,null,null,[[null,true]]],["Deuxième ligne.","Second line.",null,null,3,null,null,[[]],[[["de81a00d","en_fr_2023q1.md"]]]]],null,"en",null,null,null,0.98,[],[["en"],null,[0.98],["en"]]]',
  );
  assert.deepEqual(parseFreeResponse(data), {
    text: "Bonjour le monde. Comment vas-tu?\nDeuxième ligne.",
    detectedSource: "en",
  });
});

test("an answer without text is an error, not an empty translation", () => {
  assert.throws(() => parseFreeResponse([[], null, "en"]));
  assert.throws(() => parseFreeResponse(null));
  assert.throws(() => parseFreeResponse({}));
});
