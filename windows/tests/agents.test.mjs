// Agent sessions in the windows: avatar states, tickers, ordering, and the
// Markdown reader that keeps model output from becoming markup.
//
//   npm test

import { test } from "node:test";
import assert from "node:assert/strict";

const { botState, tickerSteps, orderSessions, shortFolder, planProgress, sessionTaskId, sessionIdOf } = await import(
  "../src/core/agents.ts"
);
const { parseMarkdown, parseInline, safeHref } = await import("../src/core/markdown.ts");

const summary = (over = {}) => ({
  id: "s1", name: "Engineer 1", role: "engineer", color: "#4F8CFF", provider: "anthropic", model: "m",
  folder: "C:/p", mode: "ask", status: "idle", error: null, parent: null, updatedAt: 0, activity: "",
  todos: [], approval: null, entries: 0, ...over,
});

test("each session state has its Mochi look", () => {
  assert.equal(botState("running"), "working");
  assert.equal(botState("waiting"), "approval");
  assert.equal(botState("done"), "finished");
  assert.equal(botState("error"), "error");
  assert.equal(botState("idle"), "idle");
});

test("the ticker shows the plan at its current step, else the latest activity", () => {
  const todos = [
    { content: "Read code", status: "completed" },
    { content: "Write test", status: "in_progress" },
    { content: "Run tests", status: "pending" },
  ];
  assert.deepEqual(tickerSteps(summary({ todos })), { steps: ["Read code", "Write test", "Run tests"], index: 1 });
  assert.equal(planProgress(todos), "1/3");
  assert.deepEqual(tickerSteps(summary({ activity: "Run npm test" })), { steps: ["Run npm test"], index: 0 });
  const waiting = summary({ status: "waiting", activity: "x", approval: { summary: "Edit a.ts" } });
  assert.deepEqual(tickerSteps(waiting).steps, ["Edit a.ts"]);
  assert.deepEqual(tickerSteps(summary()).steps, []);
});

test("sessions needing the user come first, then busy ones, then the latest", () => {
  const list = [
    summary({ id: "a", status: "done", updatedAt: 9 }),
    summary({ id: "b", status: "running", updatedAt: 1 }),
    summary({ id: "c", status: "waiting", updatedAt: 0 }),
    summary({ id: "d", status: "idle", updatedAt: 10 }),
  ];
  assert.deepEqual(orderSessions(list).map((s) => s.id), ["c", "b", "d", "a"]);
});

test("pill ids and folders read well", () => {
  assert.equal(sessionIdOf(sessionTaskId("s9")), "s9");
  assert.equal(sessionIdOf("integration_claude"), null);
  assert.equal(shortFolder("C:\\Users\\me\\code\\app"), "…/code/app");
  assert.equal(shortFolder("/tmp"), "tmp");
});

test("markdown blocks: headings, lists, code, quotes", () => {
  const md = "# Title\n\nSome **bold** and `code`.\n\n- one\n- two\n  more\n\n1. first\n2. second\n\n```ts\nconst a = 1;\n```\n\n> quoted";
  const blocks = parseMarkdown(md);
  assert.deepEqual(blocks.map((b) => b.t), ["heading", "paragraph", "list", "list", "code", "quote"]);
  assert.equal(blocks[0].level, 1);
  assert.deepEqual(blocks[1].children.map((c) => c.t), ["text", "strong", "text", "code", "text"]);
  assert.equal(blocks[2].items.length, 2);
  assert.equal(blocks[2].items[1][0].text, "two\nmore");
  assert.equal(blocks[3].ordered, true);
  assert.deepEqual(blocks[4], { t: "code", lang: "ts", text: "const a = 1;" });
});

test("only web and mail links survive; markup stays text", () => {
  assert.equal(safeHref("javascript:alert(1)"), null);
  assert.equal(safeHref(" https://x.dev "), "https://x.dev");
  const bad = parseInline("[click](javascript:alert(1))");
  assert.ok(bad.every((n) => n.t === "text"));
  const good = parseInline("see [docs](https://docs.rs) now");
  assert.equal(good[1].t, "link");
  assert.equal(good[1].href, "https://docs.rs");
  const html = parseMarkdown("<img src=x onerror=alert(1)>");
  assert.deepEqual(html, [{ t: "paragraph", children: [{ t: "text", text: "<img src=x onerror=alert(1)>" }] }]);
  // snake_case is not emphasis.
  assert.deepEqual(parseInline("a snake_case_name here").map((n) => n.t), ["text"]);
});
