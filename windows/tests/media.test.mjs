// Music tab helpers: which session is shown, and how time reads.
//
//   npm test

import { test } from "node:test";
import assert from "node:assert/strict";

const { pickSession, formatTime, livePosition } = await import("../src/core/media.ts");

const s = (id, { playing = false, current = false } = {}) => ({ id, playing, current });

test("the picked session stays picked while it exists", () => {
  const list = [s("edge", { current: true, playing: true }), s("spotify")];
  assert.equal(pickSession(list, "spotify").id, "spotify");
});

test("without a pick, the media keys' session wins, then whatever plays", () => {
  assert.equal(pickSession([s("a"), s("b", { current: true })], null).id, "b");
  assert.equal(pickSession([s("a"), s("b", { playing: true })], "gone").id, "b");
  assert.equal(pickSession([s("a"), s("b")], null).id, "a");
  assert.equal(pickSession([], "a"), null);
});

test("times read like a player's", () => {
  assert.equal(formatTime(0), "0:00");
  assert.equal(formatTime(75.9), "1:15");
  assert.equal(formatTime(3725), "1:02:05");
  assert.equal(formatTime(-3), "0:00");
});

test("a playing track moves on between polls, a paused one does not", () => {
  assert.equal(livePosition(10, 200, true, 1500), 11.5);
  assert.equal(livePosition(10, 200, false, 1500), 10);
  assert.equal(livePosition(199, 200, true, 5000), 200);
  assert.equal(livePosition(null, 200, true, 1000), null);
  assert.equal(livePosition(10, null, true, 1000), 11);
});
