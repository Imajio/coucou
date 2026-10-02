// Chat model picker: which model a provider switch lands on, and what the
// picker lists.
//
//   npm test

import { test } from "node:test";
import assert from "node:assert/strict";

const { chooseModel, switchProvider, selectModel, pickerModels, modelLabel, FALLBACK_MODELS } = await import(
  "../src/core/models.ts"
);

const m = (id, label = id) => ({ id, label });
const base = { provider: "anthropic", model: "claude-opus-5-5", providerModels: {} };

test("a provider opens on the model last used there, if it is still offered", () => {
  assert.equal(chooseModel([m("a"), m("b")], "b"), "b");
  assert.equal(chooseModel([m("a"), m("b")], "gone"), "a");
  assert.equal(chooseModel([m("a")], undefined), "a");
  assert.equal(chooseModel([], "kept"), "kept");
  assert.equal(chooseModel([], undefined), "");
});

test("switching provider remembers the model left behind and restores the one there", () => {
  const patch = switchProvider(
    { ...base, providerModels: { openai: "gpt-5-codex" } },
    "openai",
    [m("gpt-5"), m("gpt-5-codex")],
  );
  assert.deepEqual(patch, {
    provider: "openai",
    model: "gpt-5-codex",
    providerModels: { openai: "gpt-5-codex", anthropic: "claude-opus-5-5" },
  });
  const back = switchProvider({ ...base, ...patch }, "anthropic", FALLBACK_MODELS.anthropic);
  assert.equal(back.model, "claude-opus-5-5");
  assert.equal(back.providerModels.openai, "gpt-5-codex");
});

test("when a provider's list can't be fetched, the model remembered there is kept", () => {
  const away = { provider: "custom", model: "fake", providerModels: { anthropic: "claude-sonnet-5" } };
  // No list (no key yet): trust what was picked before rather than the fallback.
  assert.equal(switchProvider(away, "anthropic", [], FALLBACK_MODELS.anthropic).model, "claude-sonnet-5");
  // Nothing picked before: the fallback's first.
  assert.equal(switchProvider({ ...away, providerModels: {} }, "anthropic", [], FALLBACK_MODELS.anthropic).model, "claude-opus-5-5");
  // A real list that no longer has it wins over the memory.
  assert.equal(switchProvider(away, "anthropic", [m("claude-x")], FALLBACK_MODELS.anthropic).model, "claude-x");
});

test("picking a model remembers it for its provider", () => {
  assert.deepEqual(selectModel({ ...base, provider: "google", model: "x" }, "gemini-2.5-pro"), {
    model: "gemini-2.5-pro",
    providerModels: { google: "gemini-2.5-pro" },
  });
});

test("the picker always shows the current model, even when the list could not be fetched", () => {
  assert.deepEqual(pickerModels(null, "openai", "gpt-5").map((x) => x.id), ["gpt-5"]);
  assert.deepEqual(pickerModels([m("a")], "ollama", "llama3").map((x) => x.id), ["llama3", "a"]);
  assert.deepEqual(pickerModels([m("a"), m("b")], "ollama", "b").map((x) => x.id), ["a", "b"]);
  assert.deepEqual(pickerModels(null, "anthropic", "").map((x) => x.id), FALLBACK_MODELS.anthropic.map((x) => x.id));
  assert.deepEqual(pickerModels(null, "openrouter", ""), []);
});

test("labels come from the provider's list when it has one", () => {
  assert.equal(modelLabel([m("claude-opus-5-5", "Claude Opus 5.5")], "claude-opus-5-5"), "Claude Opus 5.5");
  assert.equal(modelLabel(null, "gpt-5"), "gpt-5");
});
