// The measure tool in the view. Click a corner, an edge, a face (or, with
// Alt, a whole part) and then another: what each is (an edge's length, an
// arc's radius and diameter, a face's area, a cylinder's radius, a part's
// volume, mass, centre of mass and moments of inertia from the engine), and
// between the two the closest distance, centre to centre, along each axis,
// and the angle. Shown on the model as dimension lines with labels, and in a
// card listing every number.

import * as THREE from "three";
import { projectUrl } from "./engine";
import { type Feature, Topology, type V3, centreOf, measureBetween, worldMesh, type Measurement } from "./measureMath";

export type MassReport = {
  volume_mm3: number;
  area_mm2: number;
  center_of_mass: V3;
  centroid: V3;
  mass_kg: number | null;
  inertia_kg_mm2: [V3, V3, V3] | null;
  principal_kg_mm2: V3 | null;
  inertia_mm5: [V3, V3, V3];
  bounds_min: V3;
  bounds_max: V3;
};

export type MeasurePick = { feature: Feature; topo: Topology | null; id: string; name: string; mass?: MassReport | null };

export type MeasureRow = { label: string; value: string };
export type MeasureCard = { title: string; rows: MeasureRow[] }[];

const ACCENT = "#e5572f";
const PICK_PX = 9;
// Rounded to −0 shows as 0.
const f = (n: number, digits = 2) => (Math.abs(n) < 0.5 * 10 ** -digits ? 0 : n).toLocaleString("en-US", { maximumFractionDigits: digits });
const mm = (n: number) => `${f(n)} mm`;

/** A part's own triangles: its mesh, or the group of a multi-part mesh the triangle is in. */
function rangeOf(mesh: THREE.Mesh, faceIndex: number) {
  const g = mesh.geometry.groups.find((gr) => faceIndex * 3 >= gr.start && faceIndex * 3 < gr.start + gr.count);
  return g ? { start: g.start, count: g.count } : undefined;
}

export class MeasureTool {
  readonly root = new THREE.Group();
  readonly labels: HTMLDivElement;
  active = false;
  picks: MeasurePick[] = [];
  result: Measurement | null = null;
  private hover: THREE.Object3D | null = null;
  private topologies = new WeakMap<THREE.Mesh, Map<string, Topology>>();
  private tags: { el: HTMLDivElement; at: THREE.Vector3 }[] = [];

  /** An item's name, for the card. */
  nameOf: (id: string) => string = (id) => id;

  constructor(
    host: HTMLElement,
    private onChange: (card: MeasureCard) => void,
    private invalidate: () => void,
  ) {
    this.labels = document.createElement("div");
    this.labels.className = "measure-labels";
    this.labels.setAttribute("aria-hidden", "true");
    host.append(this.labels);
    this.root.renderOrder = 990;
  }

  /** The feature under the pointer on a hit mesh: a corner or an edge near it on screen, else the face. */
  featureAt(hit: THREE.Intersection, pointer: THREE.Vector2, toScreen: (p: THREE.Vector3) => THREE.Vector2, wholePart: boolean): MeasurePick | null {
    const mesh = hit.object as THREE.Mesh;
    if (hit.faceIndex == null || !mesh.geometry) return null;
    mesh.updateMatrixWorld();
    const range = rangeOf(mesh, hit.faceIndex);
    const key = `${range?.start ?? 0}:${mesh.matrixWorld.elements.map((x) => x.toFixed(4)).join(",")}`;
    let cache = this.topologies.get(mesh);
    if (!cache) this.topologies.set(mesh, (cache = new Map()));
    let topo = cache.get(key);
    if (!topo) {
      cache.clear();
      cache.set(key, (topo = new Topology(worldMesh(mesh.geometry, mesh.matrixWorld, range))));
    }
    const path = mesh.userData.path as string[] | undefined;
    const id = (path?.length ? path[path.length - 1] : (mesh.userData.id as string)) ?? "";
    const name = this.nameOf(id);
    if (wholePart) {
      const all = topo.tris.map((_, i) => i);
      const c = all.reduce((s, t) => s.add(new THREE.Vector3(...topo!.points[topo!.tris[t][0]])), new THREE.Vector3()).divideScalar(Math.max(all.length, 1));
      return { feature: { kind: "part", id, name, triangles: all, centroid: [c.x, c.y, c.z] }, topo, id, name };
    }
    const tri = hit.faceIndex - (range?.start ?? 0) / 3;
    const features = topo.featureEdges();
    const screen = (i: number) => toScreen(new THREE.Vector3(...topo!.points[i]));
    // Corners: where feature edges meet other than two in a smooth line.
    const degree = new Map<number, number>();
    for (const [a, b] of features) {
      degree.set(a, (degree.get(a) ?? 0) + 1);
      degree.set(b, (degree.get(b) ?? 0) + 1);
    }
    // Only corners and edges of the face that was hit (not ones behind it).
    const face = topo.faceAt(tri);
    const on = new Set(face.triangles.flatMap((t) => topo!.tris[t]));
    let best: { at: number; d: number } | null = null;
    for (const [i, n] of degree) {
      if (n === 2 || !on.has(i)) continue;
      const d = screen(i).distanceTo(pointer);
      if (d < PICK_PX && (!best || d < best.d)) best = { at: i, d };
    }
    if (best) return { feature: { kind: "vertex", at: topo.points[best.at] }, topo, id, name };
    let edge: { a: number; b: number; d: number } | null = null;
    const ab = new THREE.Vector2();
    const ap = new THREE.Vector2();
    for (const [a, b] of features) {
      if (!on.has(a) || !on.has(b)) continue;
      const sa = screen(a);
      const sb = screen(b);
      ab.subVectors(sb, sa);
      ap.subVectors(pointer, sa);
      const t = Math.min(1, Math.max(0, ab.lengthSq() > 0 ? ap.dot(ab) / ab.lengthSq() : 0));
      const d = sa.clone().addScaledVector(ab, t).distanceTo(pointer);
      if (d < PICK_PX * 0.75 && (!edge || d < edge.d)) edge = { a, b, d };
    }
    if (edge && on.has(edge.a) && on.has(edge.b)) return { feature: topo.edgeThrough(edge.a, edge.b, features), topo, id, name };
    return { feature: face, topo, id, name };
  }

  /** Shows what a click would pick. */
  setHover(pick: MeasurePick | null) {
    if (this.hover) {
      this.root.remove(this.hover);
      disposeTree(this.hover);
      this.hover = null;
    }
    if (pick) {
      this.hover = highlight(pick, 0.35);
      this.root.add(this.hover);
    }
    this.invalidate();
  }

  /** Adds a pick: a third starts over. */
  async pick(p: MeasurePick) {
    if (this.picks.length >= 2) this.picks = [];
    this.picks.push(p);
    if (p.feature.kind === "part") {
      try {
        const r = await fetch(projectUrl(`/measure/${encodeURIComponent(p.id)}`));
        p.mass = r.ok ? ((await r.json()) as MassReport) : null;
      } catch {
        p.mass = null;
      }
    }
    this.refresh();
  }

  clear() {
    this.picks = [];
    this.result = null;
    this.setHover(null);
    this.refresh();
  }

  /** Redraws the picks and the dimension between them, and reports the numbers. */
  refresh() {
    for (const c of [...this.root.children]) if (c !== this.hover) (this.root.remove(c), disposeTree(c));
    this.labels.replaceChildren();
    this.tags = [];
    for (const p of this.picks) this.root.add(highlight(p, 0.7));
    const [a, b] = this.picks;
    this.result = a && b ? measureBetween(a.feature, a.topo, b.feature, b.topo) : null;
    const card: MeasureCard = this.picks.map((p, i) => ({ title: `${i ? "Second" : "First"}: ${describe(p.feature)}`, rows: rowsOf(p) }));
    for (const p of this.picks) {
      const tag = tagOf(p);
      if (tag) this.tag(tag, new THREE.Vector3(...centreOf(p.feature)), "is-single");
    }
    const r = this.result;
    if (r) {
      const rows: MeasureRow[] = [];
      if (r.closest) {
        rows.push({ label: "Distance (closest)", value: mm(r.closest.distance) });
        rows.push({ label: "ΔX ΔY ΔZ", value: r.closest.delta.map((d) => f(d)).join(", ") + " mm" });
        if (r.closest.distance > 1e-6) this.dimension(r.closest.from, r.closest.to, mm(r.closest.distance), ACCENT);
      }
      if (r.centres) {
        rows.push({ label: "Centre to centre", value: mm(r.centres.distance) });
        rows.push({ label: "Centres ΔX ΔY ΔZ", value: r.centres.delta.map((d) => f(d)).join(", ") + " mm" });
        if (r.centres.distance > 1e-6 && (!r.closest || Math.abs(r.centres.distance - r.closest.distance) > 1e-6)) this.dimension(r.centres.from, r.centres.to, `⊙ ${mm(r.centres.distance)}`, "#5b6b8c", true);
      }
      if (r.angle !== null) rows.push({ label: "Angle", value: `${f(r.angle, 3)}°` });
      card.push({ title: "Between them", rows });
    }
    this.onChange(card);
    this.invalidate();
  }

  /** Places the labels on screen (call each frame). */
  layout(toScreen: (p: THREE.Vector3) => THREE.Vector2) {
    for (const t of this.tags) {
      const s = toScreen(t.at);
      t.el.style.transform = `translate(${s.x}px, ${s.y}px) translate(-50%, -50%)`;
    }
  }

  private tag(text: string, at: THREE.Vector3, cls = "") {
    const el = document.createElement("div");
    el.className = `measure-tag ${cls}`;
    el.textContent = text;
    this.labels.append(el);
    this.tags.push({ el, at });
  }

  private dimension(from: V3, to: V3, text: string, color: string, dashed = false) {
    const g = new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(...from), new THREE.Vector3(...to)]);
    const m = dashed
      ? new THREE.LineDashedMaterial({ color, dashSize: 2, gapSize: 1.5, depthTest: false, transparent: true, toneMapped: false, fog: false })
      : new THREE.LineBasicMaterial({ color, depthTest: false, transparent: true, toneMapped: false, fog: false });
    const line = new THREE.Line(g, m);
    if (dashed) {
      const l = new THREE.Vector3(...from).distanceTo(new THREE.Vector3(...to));
      (m as THREE.LineDashedMaterial).dashSize = (m as THREE.LineDashedMaterial).gapSize = Math.max(l / 30, 0.2);
      line.computeLineDistances();
    }
    line.renderOrder = 995;
    line.raycast = () => {};
    const ends = new THREE.Points(g, new THREE.PointsMaterial({ color, size: 7, sizeAttenuation: false, depthTest: false, transparent: true, toneMapped: false, fog: false }));
    ends.renderOrder = 996;
    ends.raycast = () => {};
    this.root.add(line, ends);
    this.tag(text, new THREE.Vector3(...from).add(new THREE.Vector3(...to)).multiplyScalar(0.5), dashed ? "is-centres" : "");
  }

  dispose() {
    this.clear();
    this.labels.remove();
  }
}

/** What a feature is, in words. */
export function describe(f: Feature): string {
  switch (f.kind) {
    case "vertex":
      return "corner";
    case "edge":
      return f.circle ? (f.closed ? "circular edge" : "arc") : f.dir ? "straight edge" : "edge";
    case "face":
      return f.normal ? "flat face" : f.cylinder ? "cylindrical face" : "curved face";
    case "part":
      return f.name ? `part ${f.name}` : "part";
  }
}

/** The numbers one pick has on its own. */
export function rowsOf(p: MeasurePick): MeasureRow[] {
  const x = p.feature;
  const at = (v: V3) => v.map((n) => f(n)).join(", ");
  switch (x.kind) {
    case "vertex":
      return [{ label: "At", value: `${at(x.at)} mm` }];
    case "edge": {
      const rows = [{ label: "Length", value: mm(x.length) }];
      if (x.circle) rows.push({ label: "Radius", value: mm(x.circle.radius) }, { label: "Diameter", value: mm(x.circle.radius * 2) }, { label: "Centre", value: `${at(x.circle.center)} mm` });
      return rows;
    }
    case "face": {
      const rows = [{ label: "Area", value: `${f(x.area)} mm²` }];
      if (x.normal) rows.push({ label: "Normal", value: at(x.normal) });
      if (x.cylinder) rows.push({ label: "Radius", value: mm(x.cylinder.radius) }, { label: "Diameter", value: mm(x.cylinder.radius * 2) }, { label: "Axis", value: at(x.cylinder.axis) });
      return rows;
    }
    case "part": {
      const m = p.mass;
      if (!m) return [{ label: "Measuring", value: m === null ? "not available" : "…" }];
      const rows = [
        { label: "Volume", value: `${f(m.volume_mm3)} mm³` },
        { label: "Surface area", value: `${f(m.area_mm2)} mm²` },
        { label: "Mass", value: m.mass_kg === null ? "no material" : m.mass_kg < 1 ? `${f(m.mass_kg * 1000, 3)} g` : `${f(m.mass_kg, 4)} kg` },
        { label: "Centre of mass", value: `${at(m.center_of_mass)} mm` },
      ];
      if (m.inertia_kg_mm2) {
        const i = m.inertia_kg_mm2;
        rows.push({ label: "Ixx Iyy Izz", value: `${[i[0][0], i[1][1], i[2][2]].map((n) => f(n, 4)).join(", ")} kg·mm²` });
        rows.push({ label: "Ixy Ixz Iyz", value: `${[i[0][1], i[0][2], i[1][2]].map((n) => f(n, 4)).join(", ")} kg·mm²` });
      }
      if (m.principal_kg_mm2) rows.push({ label: "Principal", value: `${m.principal_kg_mm2.map((n) => f(n, 4)).join(", ")} kg·mm²` });
      return rows;
    }
  }
}

function tagOf(p: MeasurePick): string | null {
  const x = p.feature;
  if (x.kind === "edge") return x.circle ? `R ${f(x.circle.radius)} · Ø ${f(x.circle.radius * 2)}` : mm(x.length);
  if (x.kind === "face") return x.cylinder ? `R ${f(x.cylinder.radius)} · Ø ${f(x.cylinder.radius * 2)}` : `${f(x.area)} mm²`;
  if (x.kind === "part" && p.mass) return p.mass.mass_kg !== null ? `${f(p.mass.mass_kg * 1000, 2)} g` : `${f(p.mass.volume_mm3)} mm³`;
  return null;
}

/** The picked feature drawn over the model. */
function highlight(p: MeasurePick, strength: number): THREE.Object3D {
  const x = p.feature;
  const color = new THREE.Color(ACCENT);
  if (x.kind === "vertex") {
    const g = new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(...x.at)]);
    const o = new THREE.Points(g, new THREE.PointsMaterial({ color, size: 11, sizeAttenuation: false, depthTest: false, transparent: true, opacity: 0.4 + strength * 0.6, toneMapped: false, fog: false }));
    o.renderOrder = 997;
    o.raycast = () => {};
    return o;
  }
  if (x.kind === "edge") {
    const g = new THREE.BufferGeometry().setFromPoints(x.points.map((q) => new THREE.Vector3(...q)));
    const o = new THREE.Line(g, new THREE.LineBasicMaterial({ color, depthTest: false, transparent: true, opacity: 0.4 + strength * 0.6, toneMapped: false, fog: false }));
    o.renderOrder = 997;
    o.raycast = () => {};
    return o;
  }
  const topo = p.topo;
  const corners = topo ? topo.corners(x.triangles) : [];
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.Float32BufferAttribute(corners.flat(2), 3));
  const o = new THREE.Mesh(
    g,
    new THREE.MeshBasicMaterial({ color, transparent: true, opacity: strength * (x.kind === "part" ? 0.35 : 0.5), depthWrite: false, side: THREE.DoubleSide, polygonOffset: true, polygonOffsetFactor: -2, polygonOffsetUnits: -2, toneMapped: false, fog: false }),
  );
  o.renderOrder = 994;
  o.raycast = () => {};
  return o;
}

function disposeTree(o: THREE.Object3D) {
  o.traverse((c) => {
    const m = c as THREE.Mesh;
    m.geometry?.dispose();
    const mat = m.material as THREE.Material | THREE.Material[] | undefined;
    if (Array.isArray(mat)) mat.forEach((x) => x.dispose());
    else mat?.dispose();
  });
}
