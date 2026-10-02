// Cal.com calendar: half-month pages of 7-day rows, navigation, day grouping.
//
//   npm test

import { test } from "node:test";
import assert from "node:assert/strict";

const { pageFor, dayKey, monthRows, pageRows, stepPage, bookingsOn } = await import("../src/core/calendar.ts");

const days = (row) => row.map((d) => (d ? d.getDate() : null));

test("a month is cut into rows of 7 days from day 1, the last one padded", () => {
  const october = monthRows(2026, 9);
  assert.equal(october.length, 5);
  assert.deepEqual(days(october[0]), [1, 2, 3, 4, 5, 6, 7]);
  assert.deepEqual(days(october[4]), [29, 30, 31, null, null, null, null]);
  // February 2026 has 28 days: four full rows, nothing padded.
  const february = monthRows(2026, 1);
  assert.equal(february.length, 4);
  assert.deepEqual(days(february[3]), [22, 23, 24, 25, 26, 27, 28]);
  assert.equal(monthRows(2028, 1).length, 5);
});

test("Q1 shows the first two rows and Q2 the rest", () => {
  assert.deepEqual(pageRows({ year: 2026, month: 9, half: 1 }).map(days)[1], [8, 9, 10, 11, 12, 13, 14]);
  const q2 = pageRows({ year: 2026, month: 9, half: 2 });
  assert.equal(q2.length, 3);
  assert.equal(q2[0][0].getDate(), 15);
  assert.equal(pageRows({ year: 2026, month: 1, half: 2 }).length, 2);
});

test("the first page holds today", () => {
  assert.deepEqual(pageFor(new Date(2026, 9, 3)), { year: 2026, month: 9, half: 1 });
  assert.deepEqual(pageFor(new Date(2026, 9, 14)), { year: 2026, month: 9, half: 1 });
  assert.deepEqual(pageFor(new Date(2026, 9, 15)), { year: 2026, month: 9, half: 2 });
});

test("navigation moves half a month and crosses years", () => {
  assert.deepEqual(stepPage({ year: 2026, month: 9, half: 1 }, 1), { year: 2026, month: 9, half: 2 });
  assert.deepEqual(stepPage({ year: 2026, month: 11, half: 2 }, 1), { year: 2027, month: 0, half: 1 });
  assert.deepEqual(stepPage({ year: 2026, month: 9, half: 2 }, -1), { year: 2026, month: 9, half: 1 });
  assert.deepEqual(stepPage({ year: 2026, month: 0, half: 1 }, -1), { year: 2025, month: 11, half: 2 });
});

test("bookings are grouped by local day, earliest first", () => {
  assert.equal(dayKey(new Date(2026, 0, 5, 23, 59)), "2026-01-05");
  const at = (d, hh, mm) => new Date(2026, 9, d, hh, mm).toISOString();
  const bookings = [
    { id: "b", start: at(3, 15, 0) },
    { id: "x", start: at(4, 9, 0) },
    { id: "a", start: at(3, 9, 30) },
    { id: "bad", start: "not a date" },
    { id: "none" },
  ];
  assert.deepEqual(bookingsOn(bookings, "2026-10-03").map((b) => b.id), ["a", "b"]);
  assert.deepEqual(bookingsOn(bookings, "2026-10-05"), []);
});
