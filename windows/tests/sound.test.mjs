// Sound volume: the slider value and the gain it plays at.
//
//   npm test

import { test } from "node:test";
import assert from "node:assert/strict";

const { outputGain, MAX_VOLUME } = await import("../src/core/sound.ts");

test("every sound plays twice as loud as the slider value", () => {
  assert.equal(outputGain(0.12), 0.24);
  assert.equal(outputGain(MAX_VOLUME), 0.4);
  assert.equal(outputGain(0), 0);
});

test("out of range values are clamped before the boost, so nothing clips", () => {
  assert.equal(outputGain(5), 0.4);
  assert.equal(outputGain(-1), 0);
  assert.ok(outputGain(Number.MAX_VALUE) <= 1);
});
