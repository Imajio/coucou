// Active pills: what may be declared, the main pill, and the island's order.
//
//   npm test

import { test } from "node:test";
import assert from "node:assert/strict";

const { sanitizePills, togglePill, pillShown, pillStays, orderPills, PILL_CATALOG, MAX_ACTIVE } = await import(
  "../src/core/catalog.ts"
);

test("every pill id is unique and AI pills name a provider", () => {
  const ids = PILL_CATALOG.map((p) => p.id);
  assert.equal(new Set(ids).size, ids.length);
  for (const p of PILL_CATALOG.filter((p) => p.category === "ai")) assert.ok(p.provider, p.id);
  for (const p of PILL_CATALOG.filter((p) => p.category === "agent")) assert.ok(p.hookAgent, p.id);
});

test("unknown ids, duplicates and VS Code are dropped from the declared list", () => {
  const out = sanitizePills({ active: ["integration_github", "ghost", "integration_github", "integration_claude"], mainPill: "integration_claude" });
  assert.deepEqual(out.active, ["integration_github"]);
});

test("the main pill must be VS Code or a declared workspace pill", () => {
  assert.equal(sanitizePills({ active: ["agent_cursor"], mainPill: "agent_cursor" }).mainPill, "agent_cursor");
  assert.equal(sanitizePills({ active: [], mainPill: "agent_cursor" }).mainPill, "integration_claude");
  assert.equal(sanitizePills({ active: ["integration_github"], mainPill: "integration_github" }).mainPill, "integration_claude");
});

test("at most four pills besides VS Code, which never goes", () => {
  let c = { active: [], mainPill: "integration_claude" };
  for (const id of ["agent_gemini", "ai_openai", "integration_github", "integration_stripe", "integration_notion"]) {
    c = togglePill(c, id);
  }
  assert.equal(c.active.length, MAX_ACTIVE);
  assert.ok(!c.active.includes("integration_notion"));
  assert.deepEqual(togglePill(c, "integration_claude"), c);
  c = togglePill(c, "ai_openai");
  assert.ok(!c.active.includes("ai_openai"));
});

test("turning off the main pill gives VS Code the big card back", () => {
  const c = togglePill({ active: ["agent_cursor"], mainPill: "agent_cursor" }, "agent_cursor");
  assert.deepEqual(c, { active: [], mainPill: "integration_claude" });
});

test("declared pills stay when their session ends; undeclared ones go", () => {
  const c = { active: ["agent_gemini"], mainPill: "integration_claude" };
  assert.ok(pillShown(c, "integration_claude"));
  assert.ok(pillStays(c, "agent_gemini"));
  assert.ok(!pillStays(c, "agent_antigravity"));
  assert.ok(!pillStays(c, "agent_my-tool"));
});

test("island order follows the catalog, with unknown agents right after VS Code", () => {
  const t = (id) => ({ id });
  const ordered = orderPills([t("integration_github"), t("agent_my-tool"), t("agent_gemini"), t("integration_claude")]);
  assert.deepEqual(ordered.map((x) => x.id), ["integration_claude", "agent_my-tool", "agent_gemini", "integration_github"]);
});
