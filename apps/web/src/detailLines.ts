// Fine detail drawn as lines in the shaded view: an engraving 0.1 mm deep, a
// scale's graduations, lettering embossed 0.2 mm, a small step or groove.
// Seen square on with the light behind the camera none of it shades apart
// from the face it is cut into (its walls take no pixels), so the creases
// along narrow steps are drawn: sharp edges (60° and more) where one of the
// two faces meeting there is a narrow strip, at most DETAIL_STEP of the body's
// size (and never over 1 mm). Big walls, window reveals and a part's own
// outline are not detail and stay undrawn, as the shaded view has them.
//
// Found once per geometry, a slice of time per frame (a model of ten thousand
// parts does not stall its first frame); each line keeps the part (geometry
// group) it belongs to, so it is inked from that part's own material, and it
// fades in as the body grows on screen (graduations a few pixels apart would
// only smear grey).

import * as THREE from "three";

const DETAIL_ANGLE = 60;
const DETAIL_STEP = 0.015;
const DETAIL_MAX = 1; // mm
/** Milliseconds a frame may spend finding detail. */
const BUDGET = 6;
/** Pixels across where a body's detail starts to show, and where it is fully drawn. */
const FADE = [120, 360];

type Detail = { position: THREE.BufferAttribute; group: Uint16Array } | null;
const details = new WeakMap<THREE.BufferGeometry, Detail>();

/** The narrow-step creases of a geometry (null: none). */
export function detailOf(g: THREE.BufferGeometry): Detail {
  const pos = g.getAttribute("position") as THREE.BufferAttribute;
  const index = g.getIndex();
  const n = index ? index.count : pos.count;
  if (!g.boundingSphere) g.computeBoundingSphere();
  const most = Math.min(DETAIL_MAX, (g.boundingSphere?.radius ?? 0) * 2 * DETAIL_STEP);
  if (!(most > 0) || n < 6) return null;
  const at = (k: number) => (index ? index.getX(k) : k);
  // Corners welded by position (the engine splits vertices at creases).
  const q = 1e4 / most;
  const ids = new Map<string, number>();
  const weld = new Int32Array(pos.count).fill(-1);
  const corner = (k: number) => {
    const i = at(k);
    if (weld[i] < 0) {
      const key = `${Math.round(pos.getX(i) * q)},${Math.round(pos.getY(i) * q)},${Math.round(pos.getZ(i) * q)}`;
      let id = ids.get(key);
      if (id === undefined) ids.set(key, (id = i));
      weld[i] = id;
    }
    return weld[i];
  };
  const tris = Math.floor(n / 3);
  const groupOf = new Uint16Array(tris);
  for (const gr of g.groups) for (let t = Math.floor(gr.start / 3); t < Math.min(tris, (gr.start + gr.count) / 3); t++) groupOf[t] = gr.materialIndex ?? 0;
  const normals = new Float32Array(tris * 3);
  const a = new THREE.Vector3(), b = new THREE.Vector3(), c = new THREE.Vector3(), e = new THREE.Vector3(), f = new THREE.Vector3();
  /** How far triangle t's third corner stands off its edge p–r: the strip's width. */
  const width = (t: number, p: number, r: number) => {
    let o = at(3 * t);
    for (let k = 0; k < 3; k++) if (weld[at(3 * t + k)] !== p && weld[at(3 * t + k)] !== r) o = at(3 * t + k);
    a.fromBufferAttribute(pos, p);
    e.fromBufferAttribute(pos, r).sub(a);
    const len = e.length();
    return len > 0 ? f.fromBufferAttribute(pos, o).sub(a).cross(e).length() / len : 0;
  };
  const cos = Math.cos(THREE.MathUtils.degToRad(DETAIL_ANGLE));
  // An edge (welded ends) waits for the second triangle on it.
  const open = new Map<string, number>();
  const out: number[] = [];
  const groups: number[] = [];
  for (let t = 0; t < tris; t++) {
    const v = [corner(3 * t), corner(3 * t + 1), corner(3 * t + 2)];
    a.fromBufferAttribute(pos, v[0]);
    b.fromBufferAttribute(pos, v[1]);
    c.fromBufferAttribute(pos, v[2]);
    e.subVectors(b, a).cross(f.subVectors(c, a)).normalize();
    normals[3 * t] = e.x, normals[3 * t + 1] = e.y, normals[3 * t + 2] = e.z;
    for (let k = 0; k < 3; k++) {
      const p = v[k], r = v[(k + 1) % 3];
      if (p === r) continue;
      const key = p < r ? `${p}:${r}` : `${r}:${p}`;
      const other = open.get(key);
      if (other === undefined) {
        open.set(key, t);
        continue;
      }
      open.delete(key);
      const turn = normals[3 * t] * normals[3 * other] + normals[3 * t + 1] * normals[3 * other + 1] + normals[3 * t + 2] * normals[3 * other + 2];
      if (turn > cos) continue;
      if (Math.min(width(t, p, r), width(other, p, r)) > most) continue;
      out.push(pos.getX(p), pos.getY(p), pos.getZ(p), pos.getX(r), pos.getY(r), pos.getZ(r));
      groups.push(groupOf[t]);
    }
  }
  if (!out.length) return null;
  return { position: new THREE.BufferAttribute(new Float32Array(out), 3), group: Uint16Array.from(groups) };
}

/**
 * The ink a detail line takes on a material (linear colour): darker than it,
 * or lighter on a dark one. A metal shows darker than its colour (it mirrors
 * the room more than it scatters), so its ink is darker still.
 */
function inkFor(m: THREE.MeshStandardMaterial | undefined, out: THREE.Color) {
  if (!m) return out.setRGB(0.03, 0.035, 0.04);
  const c = m.color;
  const lum = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
  if (lum < 0.05) return out.setRGB(c.r + (0.3 - c.r) * 0.7, c.g + (0.31 - c.g) * 0.7, c.b + (0.33 - c.b) * 0.7);
  const k = 0.2 * (1 - 0.75 * (m.metalness ?? 0));
  return out.setRGB(c.r * k, c.g * k, c.b * k * 1.1);
}

export class DetailLines {
  private lines = new Map<THREE.Mesh, THREE.LineSegments>();
  /** True when detail is still being found: another frame is wanted. */
  pending = false;

  /** Brings detail lines to `bodies` (and takes them off every other body); call before a draw. */
  apply(bodies: THREE.Mesh[], materialsOf: (m: THREE.Mesh) => THREE.MeshStandardMaterial[], camera: THREE.Camera, heightPx: number, clipping: THREE.Plane[] | null) {
    const seen = new Set<THREE.Mesh>();
    const start = performance.now();
    this.pending = false;
    const ortho = camera as THREE.OrthographicCamera;
    const parallel = ortho.isOrthographicCamera === true;
    const pxPerUnit = parallel ? heightPx / ((ortho.top - ortho.bottom) / ortho.zoom) : heightPx / 2 / Math.tan(THREE.MathUtils.degToRad((camera as THREE.PerspectiveCamera).fov / 2));
    const eye = new THREE.Vector3().setFromMatrixPosition(camera.matrixWorld);
    const sphere = new THREE.Sphere();
    const ink = new THREE.Color();
    for (const body of bodies) {
      const g = body.geometry;
      let detail = details.get(g);
      if (detail === undefined) {
        if (performance.now() - start > BUDGET) {
          this.pending = true;
          continue;
        }
        details.set(g, (detail = detailOf(g)));
      }
      if (!detail) continue;
      seen.add(body);
      if (!g.boundingSphere) g.computeBoundingSphere();
      sphere.copy(g.boundingSphere!).applyMatrix4(body.matrixWorld);
      const px = parallel ? sphere.radius * 2 * pxPerUnit : ((sphere.radius * 2) / Math.max(sphere.center.distanceTo(eye) - sphere.radius, 1e-3)) * pxPerUnit;
      const fade = THREE.MathUtils.clamp((px - FADE[0]) / (FADE[1] - FADE[0]), 0, 1);
      let line = this.lines.get(body);
      // Out of sight (too small to show), the line is taken out of the scene,
      // so a model of thousands of detailed parts walks no extra objects.
      if (fade <= 0) {
        line?.removeFromParent();
        continue;
      }
      if (!line || line.userData.geometry !== g) {
        if (line) this.drop(line);
        const lg = new THREE.BufferGeometry();
        lg.setAttribute("position", detail.position);
        lg.setAttribute("color", new THREE.BufferAttribute(new Float32Array(detail.position.count * 3), 3));
        line = new THREE.LineSegments(lg, new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, depthWrite: false, toneMapped: false, fog: false }));
        line.userData = { geometry: g, inks: [] };
        line.raycast = () => {}; // never picked
        line.renderOrder = 1;
        this.lines.set(body, line);
      }
      if (line.parent !== body) {
        body.add(line);
        line.updateMatrixWorld(); // the scene's matrices were brought up to date before this
      }
      const material = line.material as THREE.LineBasicMaterial;
      material.opacity = 0.85 * fade;
      material.clippingPlanes = clipping;
      // Inked from the part's materials, again whenever they change.
      const materials = materialsOf(body);
      const inks = line.userData.inks as number[];
      let changed = inks.length !== materials.length * 3;
      for (let i = 0; i < materials.length && !changed; i++) {
        const c = materials[i].color;
        changed = inks[3 * i] !== c.r || inks[3 * i + 1] !== c.g || inks[3 * i + 2] !== c.b;
      }
      if (changed) {
        line.userData.inks = materials.flatMap((m) => [m.color.r, m.color.g, m.color.b]);
        const colors = line.geometry.getAttribute("color") as THREE.BufferAttribute;
        for (let i = 0; i < detail.group.length; i++) {
          inkFor(materials[detail.group[i]] ?? materials[0], ink);
          colors.setXYZ(2 * i, ink.r, ink.g, ink.b);
          colors.setXYZ(2 * i + 1, ink.r, ink.g, ink.b);
        }
        colors.needsUpdate = true;
      }
    }
    for (const [body, line] of this.lines) if (!seen.has(body)) this.drop(line, body);
  }

  private drop(line: THREE.LineSegments, body?: THREE.Mesh) {
    line.removeFromParent();
    line.geometry.dispose();
    (line.material as THREE.Material).dispose();
    if (body) this.lines.delete(body);
  }
}
