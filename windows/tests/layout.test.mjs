// Which view the island reopens on.
//
//   npm test

import { test } from "node:test";
import assert from "node:assert/strict";

const { resumeView, RESUMABLE_VIEWS } = await import("../src/core/layout.ts");

test("reopening returns to the tab that was left", () => {
  for (const tab of ["prompt", "music", "translate", "settings"]) {
    assert.equal(resumeView(tab, "overview"), tab);
  }
});

test("the overview comes back as whatever the overview is right now", () => {
  assert.equal(resumeView("overview", "empty"), "empty");
  assert.equal(resumeView("empty", "overview"), "overview");
});

test("nothing saved, or something that is not a tab, opens the overview", () => {
  assert.equal(resumeView(null, "overview"), "overview");
  assert.equal(resumeView("approval", "overview"), "overview");
  assert.equal(resumeView("greeting", "overview"), "overview");
  assert.equal(resumeView("nonsense", "overview"), "overview");
});

test("alert cards are never remembered as tabs", () => {
  for (const alert of ["approval", "question", "error", "finished", "confused", "note", "upload", "uploading", "choose", "greeting"]) {
    assert.equal(RESUMABLE_VIEWS.has(alert), false, alert);
  }
});
