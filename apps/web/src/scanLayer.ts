// A scan drawn in the model's space (project millimetres): its points, its
// mesh when it has one, each optionally coloured per point (a deviation
// map), and markers where a mesh check found problems.

import * as THREE from "three";

type V3 = [number, number, number];

export type ScanLayer = {
  /** x, y, z per point. */
  positions: Float32Array;
  /** r, g, b (0…1) per point; none: the scan's own grey. */
  colors?: Float32Array | null;
  /** The mesh, drawn see-through under the points (or solid when `meshOnly`). */
  mesh?: { positions: Float32Array; triangles: Uint32Array; colors?: Float32Array | null } | null;
  meshOnly?: boolean;
  /** Problem markers: dots and polylines (hole rims, non-manifold edges). */
  markers?: { at: V3[]; color: string; size?: number }[];
  lines?: { points: V3[]; color: string; closed?: boolean }[];
  /** Point size in pixels. */
  size?: number;
};

/** The objects drawing a scan layer (dispose with `disposeScanLayer`). */
export function buildScanLayer(layer: ScanLayer): THREE.Group {
  const g = new THREE.Group();
  g.name = "scan";
  if (layer.mesh && layer.mesh.triangles.length) {
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.BufferAttribute(layer.mesh.positions, 3));
    geo.setIndex(new THREE.BufferAttribute(layer.mesh.triangles, 1));
    if (layer.mesh.colors) geo.setAttribute("color", new THREE.BufferAttribute(layer.mesh.colors, 3));
    geo.computeVertexNormals();
    const coloured = !!layer.mesh.colors;
    const mat = new THREE.MeshLambertMaterial({
      color: coloured ? 0xffffff : 0xb9b4a6,
      vertexColors: coloured,
      side: THREE.DoubleSide,
      transparent: !layer.meshOnly,
      opacity: layer.meshOnly ? 1 : 0.55,
      depthWrite: !!layer.meshOnly,
      polygonOffset: true,
      polygonOffsetFactor: 1,
      polygonOffsetUnits: 1,
    });
    const mesh = new THREE.Mesh(geo, mat);
    mesh.raycast = () => {};
    g.add(mesh);
  }
  if (!layer.meshOnly && layer.positions.length) {
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.BufferAttribute(layer.positions, 3));
    if (layer.colors) geo.setAttribute("color", new THREE.BufferAttribute(layer.colors, 3));
    const pts = new THREE.Points(geo, new THREE.PointsMaterial({ size: layer.size ?? 2.5, sizeAttenuation: false, vertexColors: !!layer.colors, color: layer.colors ? 0xffffff : 0x55606e }));
    pts.raycast = () => {};
    g.add(pts);
  }
  for (const m of layer.markers ?? []) {
    if (!m.at.length) continue;
    const geo = new THREE.BufferGeometry().setFromPoints(m.at.map((p) => new THREE.Vector3(...p)));
    const dots = new THREE.Points(geo, new THREE.PointsMaterial({ color: m.color, size: m.size ?? 9, sizeAttenuation: false, depthTest: false, transparent: true }));
    dots.renderOrder = 5;
    dots.raycast = () => {};
    g.add(dots);
  }
  for (const l of layer.lines ?? []) {
    if (l.points.length < 2) continue;
    const pts = l.closed ? [...l.points, l.points[0]] : l.points;
    const geo = new THREE.BufferGeometry().setFromPoints(pts.map((p) => new THREE.Vector3(...p)));
    const line = new THREE.Line(geo, new THREE.LineBasicMaterial({ color: l.color, depthTest: false, transparent: true }));
    line.renderOrder = 5;
    line.raycast = () => {};
    g.add(line);
  }
  return g;
}

export function disposeScanLayer(g: THREE.Group) {
  g.traverse((o) => {
    const m = o as THREE.Mesh;
    m.geometry?.dispose();
    const mat = m.material as THREE.Material | THREE.Material[] | undefined;
    if (Array.isArray(mat)) mat.forEach((x) => x.dispose());
    else mat?.dispose();
  });
}

/** The deviation colour scale: blue (under) through green (within ±tol) to red (over), saturating at ±range. */
export const DEVIATION_STOPS = ["#2b4fd0", "#29b6d6", "#3fbf5a", "#f0c230", "#e0402f"];

export function deviationColor(d: number, tol: number, range: number, out: THREE.Color): THREE.Color {
  const stops = DEVIATION_STOPS.map((c) => new THREE.Color(c));
  // −range … −tol … 0 … +tol … +range onto the five stops.
  const r = Math.max(range, tol * 1.0001);
  const xs = [-r, -tol, 0, tol, r];
  const x = Math.max(-r, Math.min(r, d));
  let i = 0;
  while (i < 3 && x > xs[i + 1]) i++;
  const t = (x - xs[i]) / (xs[i + 1] - xs[i] || 1);
  return out.copy(stops[i]).lerp(stops[i + 1], Math.max(0, Math.min(1, t)));
}
