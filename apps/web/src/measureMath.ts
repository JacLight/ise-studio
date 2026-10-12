// Measuring what was clicked on a mesh: the B-rep-like feature under the
// pointer (a corner, an edge, a face) recovered from the triangles, and
// distances and angles between two of them. Pure geometry on world-space
// arrays, so it is tested without a browser (measureMath.test.mjs).

import * as THREE from "three";

export type V3 = [number, number, number];

export type Circle = { center: V3; radius: number; normal: V3 };

export type Feature =
  | { kind: "vertex"; at: V3 }
  | { kind: "edge"; points: V3[]; length: number; closed: boolean; dir: V3 | null; circle: Circle | null }
  | { kind: "face"; triangles: number[]; area: number; centroid: V3; normal: V3 | null; cylinder: { axis: V3; center: V3; radius: number } | null }
  | { kind: "part"; id: string; name: string; triangles: number[]; centroid: V3 };

/** A triangle mesh in world coordinates (mm): positions, and three corners per triangle. */
export type WorldMesh = { positions: Float32Array | number[]; index: ArrayLike<number> };

/** Edges sharper than this (degrees between the faces' normals) bound faces and are edges. */
export const SHARP = 20;

const v = (p: ArrayLike<number>, i: number): V3 => [p[3 * i], p[3 * i + 1], p[3 * i + 2]];
const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const add = (a: V3, b: V3): V3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const scale = (a: V3, k: number): V3 => [a[0] * k, a[1] * k, a[2] * k];
const dot = (a: V3, b: V3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const len = (a: V3) => Math.hypot(a[0], a[1], a[2]);
const unit = (a: V3): V3 => {
  const l = len(a);
  return l > 0 ? scale(a, 1 / l) : [0, 0, 0];
};
export const distance = (a: V3, b: V3) => len(sub(a, b));

/** Triangles, their normals and areas, and who borders whom across which edge. */
export class Topology {
  readonly tris: [number, number, number][] = [];
  readonly normals: V3[] = [];
  readonly areas: number[] = [];
  /** Per triangle, its three neighbours (−1: none) across edges ab, bc, ca. */
  readonly next: [number, number, number][] = [];
  /** Welded vertex id of each position index. */
  readonly weld: number[] = [];
  readonly points: V3[] = [];

  readonly mesh: WorldMesh;

  constructor(mesh: WorldMesh) {
    this.mesh = mesh;
    const p = mesh.positions;
    const ids = new Map<string, number>();
    for (let i = 0; i < p.length / 3; i++) {
      // Rounded to a micrometre; −0 and 0 weld (a seam at angle 2π meets angle 0).
      const k = `${Math.round(p[3 * i] * 1000)},${Math.round(p[3 * i + 1] * 1000)},${Math.round(p[3 * i + 2] * 1000)}`;
      let id = ids.get(k);
      if (id === undefined) {
        ids.set(k, (id = this.points.length));
        this.points.push(v(p, i));
      }
      this.weld.push(id);
    }
    const edges = new Map<string, number[]>();
    for (let t = 0; t < mesh.index.length / 3; t++) {
      const tri: [number, number, number] = [this.weld[mesh.index[3 * t]], this.weld[mesh.index[3 * t + 1]], this.weld[mesh.index[3 * t + 2]]];
      this.tris.push(tri);
      const [a, b, c] = tri.map((i) => this.points[i]);
      const n = cross(sub(b, a), sub(c, a));
      this.areas.push(len(n) / 2);
      this.normals.push(unit(n));
      for (let e = 0; e < 3; e++) {
        const key = edgeKey(tri[e], tri[(e + 1) % 3]);
        const list = edges.get(key);
        if (list) list.push(t);
        else edges.set(key, [t]);
      }
    }
    for (let t = 0; t < this.tris.length; t++) {
      const tri = this.tris[t];
      this.next.push([0, 1, 2].map((e) => (edges.get(edgeKey(tri[e], tri[(e + 1) % 3])) ?? []).find((u) => u !== t) ?? -1) as [number, number, number]);
    }
  }

  /** Degrees between two triangles' normals. */
  bend(a: number, b: number) {
    return THREE.MathUtils.radToDeg(Math.acos(Math.min(1, Math.max(-1, dot(this.normals[a], this.normals[b])))));
  }

  /** Feature edges: borders and creases sharper than SHARP, as welded vertex pairs. */
  featureEdges(): [number, number][] {
    if (this.features) return this.features;
    const out: [number, number][] = (this.features = []);
    for (let t = 0; t < this.tris.length; t++)
      for (let e = 0; e < 3; e++) {
        const u = this.next[t][e];
        if (u !== -1 && u < t) continue;
        if (u === -1 || this.bend(t, u) > SHARP) out.push([this.tris[t][e], this.tris[t][(e + 1) % 3]]);
      }
    return out;
  }

  /** The face a triangle is on: its flat region if that is a face, else its smooth region. */
  faceAt(start: number): Feature & { kind: "face" } {
    const flat = this.flood(start, (t, u) => this.bend(t, u) < 0.5 && Math.abs(dot(this.normals[start], sub(this.points[this.tris[u][0]], this.points[this.tris[start][0]]))) < 1e-3 * (1 + this.size()));
    // A flat region is a face when sharp edges bound most of it; a facet of a
    // curved face is bounded mostly by its smooth neighbours.
    let sharp = 0;
    let soft = 0;
    for (const t of flat)
      for (let e = 0; e < 3; e++) {
        const u = this.next[t][e];
        if (u !== -1 && flat.has(u)) continue;
        const l = distance(this.points[this.tris[t][e]], this.points[this.tris[t][(e + 1) % 3]]);
        if (u === -1 || this.bend(t, u) > SHARP) sharp += l;
        else soft += l;
      }
    const planar = sharp >= soft;
    const region = planar ? flat : this.flood(start, (t, u) => this.bend(t, u) < SHARP);
    const triangles = [...region];
    let area = 0;
    let c: V3 = [0, 0, 0];
    for (const t of triangles) {
      const [a, b, d] = this.tris[t].map((i) => this.points[i]);
      area += this.areas[t];
      c = add(c, scale(add(add(a, b), d), this.areas[t] / 3));
    }
    const centroid = area > 0 ? scale(c, 1 / area) : this.points[this.tris[start][0]];
    return { kind: "face", triangles, area, centroid, normal: planar ? this.normals[start] : null, cylinder: planar ? null : this.cylinderOf(triangles) };
  }

  /** A curved region's cylinder (axis, a point on it, radius) when it is one. */
  cylinderOf(triangles: number[]): { axis: V3; center: V3; radius: number } | null {
    // The normals of a cylinder all lie across its axis: the axis is the
    // direction they vary least along.
    const m = [
      [0, 0, 0],
      [0, 0, 0],
      [0, 0, 0],
    ];
    for (const t of triangles) {
      const n = this.normals[t];
      for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) m[i][j] += n[i] * n[j] * this.areas[t];
    }
    const { values, vectors } = symmetricEigen(m);
    const total = values[0] + values[1] + values[2];
    if (total <= 0 || values[0] / total > 0.02 || values[1] / total < 0.05) return null;
    const axis = unit(vectors[0]);
    const pts = new Set<number>();
    for (const t of triangles) for (const i of this.tris[t]) pts.add(i);
    const circle = fitCircle([...pts].map((i) => this.points[i]), axis);
    return circle ? { axis, center: circle.center, radius: circle.radius } : null;
  }

  /** The B-rep-like edge through a feature segment: the chain of segments it continues smoothly into. */
  edgeThrough(a: number, b: number, features: [number, number][]): Feature & { kind: "edge" } {
    const around = new Map<number, number[]>();
    for (const [p, q] of features) {
      around.set(p, [...(around.get(p) ?? []), q]);
      around.set(q, [...(around.get(q) ?? []), p]);
    }
    const walk = (from: number, to: number, stop: number) => {
      const out = [to];
      let prev = from;
      let cur = to;
      for (let guard = 0; guard < 100000; guard++) {
        if (cur === stop) break;
        const n = around.get(cur) ?? [];
        if (n.length !== 2) break;
        const nxt = n[0] === prev ? n[1] : n[0];
        // A corner (turning hard) ends the edge.
        const d1 = unit(sub(this.points[cur], this.points[prev]));
        const d2 = unit(sub(this.points[nxt], this.points[cur]));
        if (dot(d1, d2) < Math.cos(THREE.MathUtils.degToRad(45))) break;
        out.push(nxt);
        prev = cur;
        cur = nxt;
      }
      return out;
    };
    const ahead = walk(a, b, a);
    const closed = ahead[ahead.length - 1] === a;
    const behind = closed ? [] : walk(b, a, b);
    // Open: what lies behind a, then a, b and on; closed: round from a back to a.
    const ids = closed ? [a, ...ahead] : [...behind.reverse(), ...ahead];
    const points = ids.map((i) => this.points[i]);
    let length = 0;
    for (let i = 1; i < points.length; i++) length += distance(points[i - 1], points[i]);
    const span = sub(points[points.length - 1], points[0]);
    const straight = !closed && Math.abs(len(span) - length) < 1e-6 * Math.max(1, length);
    let circle: Circle | null = null;
    if (!straight && points.length >= 3) {
      const normal = planeNormal(points);
      const c = normal && fitCircle(points, normal);
      if (normal && c) circle = { center: c.center, radius: c.radius, normal };
    }
    return { kind: "edge", points, length, closed, dir: straight ? unit(span) : null, circle };
  }

  private features: [number, number][] | null = null;
  private sizeCache = -1;
  size() {
    if (this.sizeCache < 0) {
      const b = new THREE.Box3();
      for (const p of this.points) b.expandByPoint(new THREE.Vector3(...p));
      this.sizeCache = b.isEmpty() ? 0 : b.getSize(new THREE.Vector3()).length();
    }
    return this.sizeCache;
  }

  private flood(start: number, joins: (t: number, u: number) => boolean) {
    const seen = new Set([start]);
    const queue = [start];
    while (queue.length) {
      const t = queue.pop()!;
      for (const u of this.next[t]) {
        if (u === -1 || seen.has(u) || !joins(t, u)) continue;
        seen.add(u);
        queue.push(u);
      }
    }
    return seen;
  }

  /** World-space corners of triangles, for distances. */
  corners(triangles: number[]): [V3, V3, V3][] {
    return triangles.map((t) => this.tris[t].map((i) => this.points[i]) as [V3, V3, V3]);
  }
}

const edgeKey = (a: number, b: number) => (a < b ? `${a}:${b}` : `${b}:${a}`);

/** Normal of the plane points lie in (null when they are on a line). */
function planeNormal(points: V3[]): V3 | null {
  const c = scale(points.reduce(add, [0, 0, 0] as V3), 1 / points.length);
  const m = [
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
  ];
  for (const p of points) {
    const d = sub(p, c);
    for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) m[i][j] += d[i] * d[j];
  }
  const { values, vectors } = symmetricEigen(m);
  if (values[1] <= 1e-12 * Math.max(1, values[2])) return null;
  return unit(vectors[0]);
}

/** Least-squares circle through points projected across `axis`; null when they are not round. */
export function fitCircle(points: V3[], axis: V3): { center: V3; radius: number } | null {
  const n = unit(axis);
  const u = unit(Math.abs(n[0]) < 0.9 ? cross(n, [1, 0, 0]) : cross(n, [0, 1, 0]));
  const w = cross(n, u);
  const o = points[0];
  const xy = points.map((p) => [dot(sub(p, o), u), dot(sub(p, o), w)]);
  // Kåsa: x² + y² + D x + E y + F = 0.
  const A = [
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
  ];
  const B = [0, 0, 0];
  for (const [x, y] of xy) {
    const r = [x, y, 1];
    const z = -(x * x + y * y);
    for (let i = 0; i < 3; i++) {
      for (let j = 0; j < 3; j++) A[i][j] += r[i] * r[j];
      B[i] += r[i] * z;
    }
  }
  const s = solve3(A, B);
  if (!s) return null;
  const [cx, cy] = [-s[0] / 2, -s[1] / 2];
  const r2 = cx * cx + cy * cy - s[2];
  if (!(r2 > 0)) return null;
  const radius = Math.sqrt(r2);
  const worst = Math.max(...xy.map(([x, y]) => Math.abs(Math.hypot(x - cx, y - cy) - radius)));
  if (worst > Math.max(radius * 0.03, 1e-4)) return null;
  // The centre at the middle of the points along the axis.
  const along = points.reduce((k, p) => k + dot(sub(p, o), n), 0) / points.length;
  return { center: add(add(o, add(scale(u, cx), scale(w, cy))), scale(n, along)), radius };
}

function solve3(A: number[][], b: number[]): number[] | null {
  const det = (m: number[][]) => m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
  const d = det(A);
  if (Math.abs(d) < 1e-18) return null;
  return [0, 1, 2].map((k) => det(A.map((row, i) => row.map((x, j) => (j === k ? b[i] : x)))) / d);
}

/** Eigenvalues (ascending) and eigenvectors of a symmetric 3×3 matrix. */
export function symmetricEigen(m: number[][]): { values: number[]; vectors: V3[] } {
  const a = m.map((r) => [...r]);
  const V = [
    [1, 0, 0],
    [0, 1, 0],
    [0, 0, 1],
  ];
  for (let sweep = 0; sweep < 50; sweep++) {
    const off = Math.abs(a[0][1]) + Math.abs(a[0][2]) + Math.abs(a[1][2]);
    if (off < 1e-14 * (Math.abs(a[0][0]) + Math.abs(a[1][1]) + Math.abs(a[2][2]) + 1e-300)) break;
    for (const [p, q] of [
      [0, 1],
      [0, 2],
      [1, 2],
    ]) {
      if (Math.abs(a[p][q]) < 1e-300) continue;
      const theta = (a[q][q] - a[p][p]) / (2 * a[p][q]);
      const t = theta === 0 ? 1 : Math.sign(theta) / (Math.abs(theta) + Math.sqrt(theta * theta + 1));
      const c = 1 / Math.sqrt(t * t + 1);
      const s = t * c;
      for (let k = 0; k < 3; k++) {
        const [kp, kq] = [a[k][p], a[k][q]];
        a[k][p] = c * kp - s * kq;
        a[k][q] = s * kp + c * kq;
      }
      for (let k = 0; k < 3; k++) {
        const [pk, qk] = [a[p][k], a[q][k]];
        a[p][k] = c * pk - s * qk;
        a[q][k] = s * pk + c * qk;
      }
      for (let k = 0; k < 3; k++) {
        const [kp, kq] = [V[k][p], V[k][q]];
        V[k][p] = c * kp - s * kq;
        V[k][q] = s * kp + c * kq;
      }
    }
  }
  const order = [0, 1, 2].sort((i, j) => a[i][i] - a[j][j]);
  return { values: order.map((i) => a[i][i]), vectors: order.map((i) => [V[0][i], V[1][i], V[2][i]] as V3) };
}

// ---- Distances ---------------------------------------------------------------

/** What a feature is made of, for distances: points, segments and triangles. */
export type Shape = { points: V3[]; segments: [V3, V3][]; triangles: [V3, V3, V3][] };

/** Closest point on segment ab to p. */
function onSegment(p: V3, a: V3, b: V3): V3 {
  const ab = sub(b, a);
  const l2 = dot(ab, ab);
  const t = l2 > 0 ? Math.min(1, Math.max(0, dot(sub(p, a), ab) / l2)) : 0;
  return add(a, scale(ab, t));
}

/** Closest point on triangle abc to p (Ericson, Real-Time Collision Detection 5.1.5). */
export function onTriangle(p: V3, a: V3, b: V3, c: V3): V3 {
  const ab = sub(b, a);
  const ac = sub(c, a);
  const ap = sub(p, a);
  const d1 = dot(ab, ap);
  const d2 = dot(ac, ap);
  if (d1 <= 0 && d2 <= 0) return a;
  const bp = sub(p, b);
  const d3 = dot(ab, bp);
  const d4 = dot(ac, bp);
  if (d3 >= 0 && d4 <= d3) return b;
  const vc = d1 * d4 - d3 * d2;
  if (vc <= 0 && d1 >= 0 && d3 <= 0) return add(a, scale(ab, d1 / (d1 - d3)));
  const cp = sub(p, c);
  const d5 = dot(ab, cp);
  const d6 = dot(ac, cp);
  if (d6 >= 0 && d5 <= d6) return c;
  const vb = d5 * d2 - d1 * d6;
  if (vb <= 0 && d2 >= 0 && d6 <= 0) return add(a, scale(ac, d2 / (d2 - d6)));
  const va = d3 * d6 - d5 * d4;
  if (va <= 0 && d4 - d3 >= 0 && d5 - d6 >= 0) return add(b, scale(sub(c, b), (d4 - d3) / (d4 - d3 + (d5 - d6))));
  const denom = 1 / (va + vb + vc);
  return add(a, add(scale(ab, vb * denom), scale(ac, vc * denom)));
}

/** Closest points between segments p1q1 and p2q2. */
function segmentSegment(p1: V3, q1: V3, p2: V3, q2: V3): [V3, V3] {
  const d1 = sub(q1, p1);
  const d2 = sub(q2, p2);
  const r = sub(p1, p2);
  const a = dot(d1, d1);
  const e = dot(d2, d2);
  const f = dot(d2, r);
  let s = 0;
  let t = 0;
  if (a <= 1e-12 && e <= 1e-12) return [p1, p2];
  if (a <= 1e-12) t = Math.min(1, Math.max(0, f / e));
  else {
    const c = dot(d1, r);
    if (e <= 1e-12) s = Math.min(1, Math.max(0, -c / a));
    else {
      const b = dot(d1, d2);
      const denom = a * e - b * b;
      s = denom !== 0 ? Math.min(1, Math.max(0, (b * f - c * e) / denom)) : 0;
      t = (b * s + f) / e;
      if (t < 0) {
        t = 0;
        s = Math.min(1, Math.max(0, -c / a));
      } else if (t > 1) {
        t = 1;
        s = Math.min(1, Math.max(0, (b - c) / a));
      }
    }
  }
  return [add(p1, scale(d1, s)), add(p2, scale(d2, t))];
}

/** Whether segment pq passes through triangle abc, and where. */
function segmentHitsTriangle(p: V3, q: V3, a: V3, b: V3, c: V3): V3 | null {
  const n = cross(sub(b, a), sub(c, a));
  const dp = dot(sub(p, a), n);
  const dq = dot(sub(q, a), n);
  if (dp * dq > 0 || dp === dq) return null;
  const x = add(p, scale(sub(q, p), dp / (dp - dq)));
  const inside = [
    [a, b],
    [b, c],
    [c, a],
  ].every(([s, e]) => dot(cross(sub(e, s), sub(x, s)), n) >= -1e-9);
  return inside ? x : null;
}

/** The two closest points of two shapes. */
export function closest(A: Shape, B: Shape): { from: V3; to: V3; distance: number } {
  let best = { from: [0, 0, 0] as V3, to: [0, 0, 0] as V3, distance: Infinity };
  const take = (from: V3, to: V3) => {
    const d = distance(from, to);
    if (d < best.distance) best = { from, to, distance: d };
  };
  const segsOf = (s: Shape) => [...s.segments, ...s.triangles.flatMap(([a, b, c]) => [[a, b], [b, c], [c, a]] as [V3, V3][])];
  const ptsOf = (s: Shape) => [...s.points, ...s.segments.flat(), ...s.triangles.flat()];
  for (const p of ptsOf(A)) {
    for (const q of B.points) take(p, q);
    for (const [a, b] of B.segments) take(p, onSegment(p, a, b));
    for (const [a, b, c] of B.triangles) take(p, onTriangle(p, a, b, c));
  }
  for (const q of ptsOf(B)) {
    for (const [a, b] of A.segments) take(onSegment(q, a, b), q);
    for (const [a, b, c] of A.triangles) take(onTriangle(q, a, b, c), q);
  }
  const sa = segsOf(A);
  const sb = segsOf(B);
  if (sa.length * sb.length < 4e6)
    for (const [a, b] of sa)
      for (const [c, d] of sb) {
        const [x, y] = segmentSegment(a, b, c, d);
        take(x, y);
      }
  // Crossing through: touching shapes are 0 apart.
  if (best.distance > 0)
    for (const [p, q] of A.segments.length ? A.segments : sa)
      for (const [a, b, c] of B.triangles) {
        const x = segmentHitsTriangle(p, q, a, b, c);
        if (x) take(x, x);
      }
  return best;
}

/** The pieces of a feature, for distances. */
export function shapeOf(f: Feature, topo: Topology | null): Shape {
  if (f.kind === "vertex") return { points: [f.at], segments: [], triangles: [] };
  if (f.kind === "edge") return { points: [], segments: f.points.slice(1).map((p, i) => [f.points[i], p] as [V3, V3]), triangles: [] };
  return { points: [], segments: [], triangles: topo ? topo.corners(f.triangles) : [] };
}

/** A feature's centre: a circle's or cylinder's centre, else its middle. */
export function centreOf(f: Feature): V3 {
  if (f.kind === "vertex") return f.at;
  if (f.kind === "edge") {
    if (f.circle) return f.circle.center;
    let total = 0;
    let c: V3 = [0, 0, 0];
    for (let i = 1; i < f.points.length; i++) {
      const l = distance(f.points[i - 1], f.points[i]);
      total += l;
      c = add(c, scale(add(f.points[i - 1], f.points[i]), l / 2));
    }
    return total > 0 ? scale(c, 1 / total) : f.points[0];
  }
  if (f.kind === "face") return f.cylinder ? f.cylinder.center : f.centroid;
  return f.centroid;
}

/** A feature's direction, for angles: a straight edge's, a flat face's normal, a cylinder's axis. */
export function directionOf(f: Feature): { dir: V3; kind: "line" | "normal" } | null {
  if (f.kind === "edge" && f.dir) return { dir: f.dir, kind: "line" };
  if (f.kind === "edge" && f.circle) return { dir: f.circle.normal, kind: "normal" };
  if (f.kind === "face" && f.normal) return { dir: f.normal, kind: "normal" };
  if (f.kind === "face" && f.cylinder) return { dir: f.cylinder.axis, kind: "line" };
  return null;
}

/** Degrees between two features (lines and planes), or null when either has no direction. */
export function angleBetween(a: Feature, b: Feature): number | null {
  const da = directionOf(a);
  const db = directionOf(b);
  if (!da || !db) return null;
  const c = Math.min(1, Math.abs(dot(da.dir, db.dir)));
  const deg = THREE.MathUtils.radToDeg(Math.acos(c));
  // A line against a plane: the angle to the plane, not to its normal.
  if (da.kind !== db.kind) return 90 - deg;
  // Two faces: the angle between their normals, 0–180 (a box's sides: 90).
  if (da.kind === "normal" && db.kind === "normal") return THREE.MathUtils.radToDeg(Math.acos(Math.min(1, Math.max(-1, dot(da.dir, db.dir)))));
  return deg;
}

export type Measurement = {
  /** Closest points and the distance between them (mm). */
  closest: { from: V3; to: V3; distance: number; delta: V3 } | null;
  /** Centre to centre (mm) and along each axis. */
  centres: { from: V3; to: V3; distance: number; delta: V3 } | null;
  angle: number | null;
};

/** Everything measured between two features. */
export function measureBetween(a: Feature, ta: Topology | null, b: Feature, tb: Topology | null): Measurement {
  const near = closest(shapeOf(a, ta), shapeOf(b, tb));
  const ca = centreOf(a);
  const cb = centreOf(b);
  return {
    closest: Number.isFinite(near.distance) ? { ...near, delta: sub(near.to, near.from) } : null,
    centres: { from: ca, to: cb, distance: distance(ca, cb), delta: sub(cb, ca) },
    angle: angleBetween(a, b),
  };
}

/** World-space triangles of a mesh (its matrixWorld applied), for a Topology. */
export function worldMesh(geometry: THREE.BufferGeometry, matrix: THREE.Matrix4, range?: { start: number; count: number }): WorldMesh {
  const pos = geometry.getAttribute("position") as THREE.BufferAttribute;
  const positions = new Float32Array(pos.count * 3);
  const p = new THREE.Vector3();
  for (let i = 0; i < pos.count; i++) {
    p.fromBufferAttribute(pos, i).applyMatrix4(matrix);
    positions[3 * i] = p.x;
    positions[3 * i + 1] = p.y;
    positions[3 * i + 2] = p.z;
  }
  const idx = geometry.getIndex();
  const all = idx ? (idx.array as ArrayLike<number>) : Array.from({ length: pos.count }, (_, i) => i);
  const start = range?.start ?? 0;
  const count = range?.count ?? all.length - start;
  const index = Array.from({ length: count }, (_, i) => all[start + i]);
  return { positions, index };
}
