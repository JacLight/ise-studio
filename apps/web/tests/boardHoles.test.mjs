// node --test apps/web/tests/boardHoles.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { splitHoles, CUT_MAX } from "../src/boardHoles.ts";

const hole = (x, y, w, h = w, plated = true) => ({ at: [x, y], size: [w, h], rot: 0, plated });

test("vias and small plated holes are painted, the rest go through", () => {
  const holes = [hole(1, 1, 2.2), hole(2, 2, 0.2), hole(3, 3, 0.65, 0.65, false), hole(4, 4, 0.6, 1.7), hole(5, 5, 1.0)];
  const vias = Array.from({ length: 400 }, (_, i) => [i * 0.1, 1, 0.6, 0.3]);
  const { cut, painted } = splitHoles(holes, vias);
  assert.deepEqual(cut.map((h) => h.at[0]).sort(), [1, 3, 4, 5], "mounting hole, unplated hole, slot and 1 mm hole are cut");
  assert.equal(painted.length, 401, "400 vias and the 0.2 mm thermal via are painted");
  assert.deepEqual(painted.find((p) => p.at[0] === 2), { at: [2, 2], d: 0.2 });
  assert.equal(painted.find((p) => p.at[0] === 0.5).d, 0.3, "a via is painted at its drill");
});

test("never more than CUT_MAX holes are cut, the largest kept", () => {
  const holes = Array.from({ length: 500 }, (_, i) => hole(i, 0, 1 + (i % 7) * 0.1));
  const { cut, painted } = splitHoles(holes, []);
  assert.equal(cut.length, CUT_MAX);
  assert.equal(painted.length, 500 - CUT_MAX);
  assert.ok(Math.min(...cut.map((h) => h.size[0])) >= Math.max(...painted.map((p) => p.d)));
});
