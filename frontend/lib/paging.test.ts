import assert from "node:assert/strict";
import { test } from "node:test";
import { cursorsFor, FIRST_PAGE, shouldPop } from "./paging.ts";

test("pops when the page of the current cursor is empty", () => {
  assert.equal(shouldPop({ cursor: "b", agvIds: [] }, "b", 3), true);
});

test("does not pop again on a stale empty page after going back", () => {
  // Cursor stack went from [_, a, b] to [_, a]; the page is still the empty one for "b".
  assert.equal(shouldPop({ cursor: "b", agvIds: [] }, "a", 2), false);
});

test("never pops from the first page or a filled page", () => {
  assert.equal(shouldPop({ cursor: undefined, agvIds: [] }, undefined, 1), false);
  assert.equal(shouldPop({ cursor: "a", agvIds: ["x"] }, "a", 2), false);
  assert.equal(shouldPop(undefined, "a", 2), false);
});

test("a cursor stack from another filter is ignored", () => {
  const state = { key: "old", stack: [undefined, "a"] };
  assert.equal(cursorsFor(state, "old"), state.stack);
  assert.equal(cursorsFor(state, "new"), FIRST_PAGE);
});
