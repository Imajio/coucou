// Island state machine: visibility rules (always visible, hover delay, reveals).
// Runs on fake timers, so every delay is checked to the millisecond without waiting.
//
//   npm test

import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";

// ── Fake timers: the FSM only uses window.setTimeout / clearTimeout ───────────

let now = 0;
let nextId = 1;
const timers = new Map();
globalThis.window = {
  setTimeout(fn, ms) {
    const id = nextId++;
    timers.set(id, { at: now + ms, fn });
    return id;
  },
  clearTimeout(id) {
    timers.delete(id);
  },
};

/** Moves the clock forward, firing due timers in order. */
function advance(seconds) {
  const end = now + seconds * 1000;
  for (;;) {
    let next = null;
    for (const [id, t] of timers) if (t.at <= end && (!next || t.at < next[1].at)) next = [id, t];
    if (!next) break;
    timers.delete(next[0]);
    now = next[1].at;
    next[1].fn();
  }
  now = end;
}

const { IslandStateMachine } = await import("../src/island/fsm.ts");

let cursorOnIsland = false;

/** A machine wired the way Island.wireFsm wires it. */
function machine({ always = false, delay = 1 } = {}) {
  const fsm = new IslandStateMachine();
  fsm.alwaysVisible = always;
  fsm.hoverRevealDelay = delay;
  fsm.onTransition = (_from, to) => {
    if (to === "petit" && !cursorOnIsland) fsm.mouseLeft();
  };
  return fsm;
}

/** Launch, then the greeting ends with the cursor elsewhere. */
function greeted(fsm) {
  fsm.launch();
  fsm.mouseLeft();
  return fsm;
}

beforeEach(() => {
  timers.clear();
  cursorOnIsland = false;
});

// ── Always visible: off ───────────────────────────────────────────────────────

test("a hover shorter than the delay never reveals", () => {
  const fsm = machine();
  fsm.mouseEntered();
  advance(0.9);
  fsm.mouseLeft();
  advance(5);
  assert.equal(fsm.state, "hidden");
});

test("resting for the delay reveals the compact island", () => {
  const fsm = machine();
  fsm.mouseEntered();
  advance(0.99);
  assert.equal(fsm.state, "hidden");
  advance(0.02);
  assert.equal(fsm.state, "petit");
});

test("a delay of 0 reveals at once", () => {
  const fsm = machine({ delay: 0 });
  fsm.mouseEntered();
  assert.equal(fsm.state, "petit");
});

test("the island stays while hovered and hides 3 s after the cursor leaves", () => {
  const fsm = machine();
  fsm.mouseEntered();
  advance(1);
  cursorOnIsland = true;
  fsm.mouseEntered();
  advance(30);
  assert.equal(fsm.state, "petit");
  cursorOnIsland = false;
  fsm.mouseLeft();
  advance(2.9);
  assert.equal(fsm.state, "petit");
  advance(0.2);
  assert.equal(fsm.state, "hidden");
});

test("routine work events do not reveal", () => {
  const fsm = machine();
  fsm.reveal();
  advance(10);
  assert.equal(fsm.state, "hidden");
});

test("an urgent event reveals and stays petitToHiddenDelay", () => {
  const fsm = machine();
  fsm.reveal(true);
  assert.equal(fsm.state, "petit");
  advance(59);
  assert.equal(fsm.state, "petit");
  advance(2);
  assert.equal(fsm.state, "hidden");
});

test("an alert still opens the island", () => {
  const fsm = machine();
  fsm.forceHome();
  assert.equal(fsm.state, "home");
});

test("the launch greeting ends hidden", () => {
  const fsm = greeted(machine());
  assert.equal(fsm.state, "petit");
  advance(3.1);
  assert.equal(fsm.state, "hidden");
});

test("an alert cancels a pending hover reveal", () => {
  const fsm = machine();
  fsm.mouseEntered();
  fsm.forceHome();
  advance(5);
  assert.equal(fsm.state, "home");
});

// ── Always visible: on ────────────────────────────────────────────────────────

test("the compact island never hides on its own", () => {
  const fsm = greeted(machine({ always: true }));
  advance(600);
  assert.equal(fsm.state, "petit");
  fsm.forceHome();
  fsm.forcePetit();
  fsm.mouseLeft();
  advance(600);
  assert.equal(fsm.state, "petit");
});

test("after a pause, a work event brings it back for good", () => {
  const fsm = greeted(machine({ always: true }));
  fsm.forceHidden();
  fsm.reveal();
  assert.equal(fsm.state, "petit");
  advance(600);
  assert.equal(fsm.state, "petit");
});

// ── Switching while running ───────────────────────────────────────────────────

test("switching on shows a hidden island, unless paused", () => {
  const shown = greeted(machine());
  advance(4);
  assert.equal(shown.state, "hidden");
  shown.setAlwaysVisible(true, false, false);
  assert.equal(shown.state, "petit");
  advance(600);
  assert.equal(shown.state, "petit");

  const paused = greeted(machine());
  advance(4);
  paused.setAlwaysVisible(true, false, true);
  assert.equal(paused.state, "hidden");
});

test("switching off hides after the linger, unless hovered", () => {
  const away = greeted(machine({ always: true }));
  away.setAlwaysVisible(false, false, false);
  advance(3.1);
  assert.equal(away.state, "hidden");

  const hovered = greeted(machine({ always: true }));
  hovered.setAlwaysVisible(false, true, false);
  advance(100);
  assert.equal(hovered.state, "petit");
});

test("switching before launch only sets the flag", () => {
  const fsm = machine();
  fsm.setAlwaysVisible(true, false, false);
  assert.equal(fsm.state, "hidden");
  fsm.launch();
  assert.equal(fsm.state, "coucou");
});

test("re-applying the same value changes nothing", () => {
  const hidden = greeted(machine());
  advance(4);
  hidden.setAlwaysVisible(false, false, false);
  assert.equal(hidden.state, "hidden");

  const paused = greeted(machine({ always: true }));
  paused.forceHidden();
  paused.setAlwaysVisible(true, false, false);
  assert.equal(paused.state, "hidden");
});

// ── Typing in the open island ─────────────────────────────────────────────────

test("typing restarts the auto-close countdown of the open island", () => {
  const fsm = machine();
  fsm.homeToPetitDelay = 5;
  fsm.forceHome();
  fsm.mouseLeft(); // the pointer is elsewhere: the countdown runs
  advance(4);
  fsm.activity();
  advance(4);
  assert.equal(fsm.state, "home");
  advance(1.1);
  assert.equal(fsm.state, "petit");
});

test("typing does not start a countdown that was not running", () => {
  const fsm = machine();
  cursorOnIsland = true;
  fsm.homeToPetitDelay = 5;
  fsm.forceHome();
  fsm.mouseEntered();
  fsm.activity();
  advance(60);
  assert.equal(fsm.state, "home");
});
