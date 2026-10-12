// node --test apps/web/src/measureMath.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { Topology, measureBetween, fitCircle, centreOf } from "./measureMath.ts";

/** A 60×40×20 box from (10, −5, 0), as the viewport has it (BoxGeometry: split normals, 24 corners). */
function box() {
  const g = new THREE.BoxGeometry(60, 40, 20).translate(40, 15, 10);
  return new Topology({ positions: g.getAttribute("position").array, index: g.getIndex().array });
}
const near = (a, b, tol = 1e-6) => assert.ok(Math.abs(a - b) <= tol * Math.max(1, Math.abs(b)), `${a} ≠ ${b}`);
/** The triangle whose normal is n. */
const faceWith = (t, n) => t.normals.findIndex((m) => Math.abs(m[0] - n[0]) + Math.abs(m[1] - n[1]) + Math.abs(m[2] - n[2]) < 1e-9);

test("faces of a 60×40×20 box: areas 2400, 1200 and 800", () => {
  const t = box();
  const top = t.faceAt(faceWith(t, [0, 0, 1]));
  near(top.area, 2400);
  assert.deepEqual(top.normal.map((x) => x + 0), [0, 0, 1]);
  near(top.centroid[0], 40);
  near(top.centroid[2], 20);
  near(t.faceAt(faceWith(t, [0, -1, 0])).area, 1200);
  near(t.faceAt(faceWith(t, [1, 0, 0])).area, 800);
});

test("top to bottom 20 mm, side to side 60, angle between adjacent faces 90°, opposite 180°", () => {
  const t = box();
  const top = t.faceAt(faceWith(t, [0, 0, 1]));
  const bottom = t.faceAt(faceWith(t, [0, 0, -1]));
  const m = measureBetween(top, t, bottom, t);
  near(m.closest.distance, 20);
  near(m.centres.distance, 20);
  near(m.centres.delta[2], -20);
  near(m.angle, 180);
  const left = t.faceAt(faceWith(t, [-1, 0, 0]));
  const right = t.faceAt(faceWith(t, [1, 0, 0]));
  near(measureBetween(left, t, right, t).closest.distance, 60);
  near(measureBetween(top, t, right, t).angle, 90);
  near(measureBetween(top, t, right, t).closest.distance, 0);
});

test("edges: a 60 mm edge; corner to corner across the box is √(60²+40²+20²)", () => {
  const t = box();
  const features = t.featureEdges();
  assert.equal(features.length, 12);
  const long = features.find(([a, b]) => Math.abs(Math.hypot(...t.points[a].map((x, i) => x - t.points[b][i])) - 60) < 1e-6);
  const edge = t.edgeThrough(long[0], long[1], features);
  near(edge.length, 60);
  assert.ok(edge.dir && Math.abs(edge.dir[0]) === 1);
  const lo = { kind: "vertex", at: [10, -5, 0] };
  const hi = { kind: "vertex", at: [70, 35, 20] };
  near(measureBetween(lo, null, hi, null).closest.distance, Math.hypot(60, 40, 20));
  // An edge against a face it lies along: parallel (0°).
  const top = t.faceAt(faceWith(t, [0, 0, 1]));
  near(measureBetween(edge, t, top, t).angle, 0);
});

test("a cylinder's face and rim: radius 10 (Ø20), rim length as drawn", () => {
  const g = new THREE.CylinderGeometry(10, 10, 30, 48).rotateX(Math.PI / 2);
  const t = new Topology({ positions: g.getAttribute("position").array, index: g.getIndex().array });
  // A side triangle: normal across the axis.
  const side = t.normals.findIndex((n) => Math.abs(n[2]) < 1e-6);
  const face = t.faceAt(side);
  assert.equal(face.normal, null);
  assert.ok(face.cylinder, "found a cylinder");
  near(face.cylinder.radius, 10, 1e-5);
  near(Math.abs(face.cylinder.axis[2]), 1, 1e-9);
  const features = t.featureEdges();
  const rim = features.find(([a, b]) => t.points[a][2] > 14 && t.points[b][2] > 14);
  const edge = t.edgeThrough(rim[0], rim[1], features);
  assert.ok(edge.closed);
  assert.ok(edge.circle);
  near(edge.circle.radius, 10, 1e-5);
  near(edge.length, 48 * 2 * 10 * Math.sin(Math.PI / 48), 1e-5);
  near(Math.abs(centreOf(edge)[2]), 15, 1e-5);
});

test("fitCircle refuses points that are not round", () => {
  assert.equal(fitCircle([[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0], [5, 2, 0]], [0, 0, 1]), null);
});
