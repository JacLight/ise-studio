// Display modes for the model: shaded, shaded with edges, wireframe (edges
// only), hidden line (edges, with what is behind a face hidden) and x-ray
// (see-through bodies, edges on top). The look each part was given is left
// alone in shaded mode; the other modes adjust its materials each frame (so
// a part that arrives or is repainted follows the mode) and put back exactly
// what they changed when the mode goes back to shaded.
//
// Three more modes analyse surfaces, as a surfacing designer reads them:
// zebra stripes (light bars reflected in the surface: stripes that run on
// unbroken across a seam show tangency, G1; stripes that also keep their
// direction show curvature continuity, G2), a curvature map (blue concave,
// green flat, red convex, from the mesh's normals), and a draft map for a
// mould pulled up Z (green releases, red faces back, yellow within a degree
// of square: needs draft).

import * as THREE from "three";
import { DetailLines } from "./detailLines";

export type DisplayMode = "shaded" | "edges" | "wireframe" | "hidden" | "xray" | "zebra" | "curvature" | "draft";

export const DISPLAY_MODES: { mode: DisplayMode; label: string }[] = [
  { mode: "shaded", label: "Shaded" },
  { mode: "edges", label: "Shaded with edges" },
  { mode: "wireframe", label: "Wireframe" },
  { mode: "hidden", label: "Hidden line" },
  { mode: "xray", label: "X-ray" },
];

/** Surface analysis shading (not in the W cycle). */
export const ANALYSIS_MODES: { mode: DisplayMode; label: string }[] = [
  { mode: "zebra", label: "Zebra stripes (continuity)" },
  { mode: "curvature", label: "Curvature map" },
  { mode: "draft", label: "Draft angle (pull +Z)" },
];
const ANALYSIS = new Set<DisplayMode>(["zebra", "curvature", "draft"]);

/** The fragment colour each analysis mode replaces the lit colour with (view-space normal `normal`). */
const ANALYSIS_GLSL: Record<string, string> = {
  zebra: `
    vec3 rv = reflect(-normalize(vViewPosition), normalize(normal));
    vec3 rw = normalize((vec4(rv, 0.0) * viewMatrix).xyz);
    float band = fract(acos(clamp(rw.z, -1.0, 1.0)) / 3.14159265 * 14.0);
    float w = fwidth(band) * 1.5;
    float stripe = smoothstep(0.5 - w, 0.5 + w, band) - smoothstep(1.0 - w, 1.0, band);
    gl_FragColor = vec4(vec3(mix(0.06, 0.97, stripe)), 1.0);`,
  curvature: `
    float t = clamp(vCurvature, -1.0, 1.0);
    vec3 c = t < 0.0 ? mix(vec3(0.15, 0.75, 0.35), vec3(0.15, 0.35, 0.95), -t) : mix(vec3(0.15, 0.75, 0.35), vec3(0.95, 0.2, 0.15), t);
    float lit = 0.55 + 0.45 * abs(normalize(normal).z);
    gl_FragColor = vec4(c * lit, 1.0);`,
  draft: `
    vec3 nw = normalize((vec4(normalize(normal), 0.0) * viewMatrix).xyz);
    float a = degrees(asin(clamp(nw.z, -1.0, 1.0)));
    vec3 c = a > 1.0 ? vec3(0.2, 0.72, 0.3) : a < -1.0 ? vec3(0.85, 0.22, 0.18) : vec3(0.98, 0.8, 0.15);
    float lit = 0.6 + 0.4 * abs(normalize(normal).z);
    gl_FragColor = vec4(c * lit, 1.0);`,
};

function analysisPatch(mode: string) {
  return (shader: THREE.WebGLProgramParametersWithUniforms) => {
    if (mode === "curvature") {
      shader.vertexShader = shader.vertexShader.replace("#include <common>", "#include <common>\nattribute float curvature;\nvarying float vCurvature;").replace("#include <begin_vertex>", "#include <begin_vertex>\nvCurvature = curvature;");
      shader.fragmentShader = shader.fragmentShader.replace("#include <common>", "#include <common>\nvarying float vCurvature;");
    }
    shader.fragmentShader = shader.fragmentShader.replace("#include <dithering_fragment>", `#include <dithering_fragment>\n${ANALYSIS_GLSL[mode]}`);
  };
}

/**
 * Mean curvature per vertex from the mesh (each edge's normal curvature,
 * (n_i − n_j)·(p_i − p_j)/|p_i − p_j|², averaged round the vertex), scaled
 * to −1…1 for the colour map: ±1 where the bend radius is a tenth of the
 * body's size or tighter.
 */
const curvatureDone = new WeakSet<THREE.BufferGeometry>();
function ensureCurvature(g: THREE.BufferGeometry) {
  if (curvatureDone.has(g) && g.getAttribute("curvature")) return;
  const pos = g.getAttribute("position") as THREE.BufferAttribute;
  const nor = g.getAttribute("normal") as THREE.BufferAttribute | undefined;
  const n = pos.count;
  const out = new Float32Array(n);
  if (nor) {
    const index = g.getIndex();
    const sum = new Float64Array(n);
    const cnt = new Uint32Array(n);
    const tri = index ? index.count / 3 : n / 3;
    const at = (k: number) => (index ? index.getX(k) : k);
    const edge = (i: number, j: number) => {
      const dx = pos.getX(i) - pos.getX(j), dy = pos.getY(i) - pos.getY(j), dz = pos.getZ(i) - pos.getZ(j);
      const l2 = dx * dx + dy * dy + dz * dz;
      if (l2 < 1e-18) return;
      const k = ((nor.getX(i) - nor.getX(j)) * dx + (nor.getY(i) - nor.getY(j)) * dy + (nor.getZ(i) - nor.getZ(j)) * dz) / l2;
      sum[i] += k, sum[j] += k, cnt[i]++, cnt[j]++;
    };
    for (let t = 0; t < tri; t++) {
      const [a, b, c] = [at(3 * t), at(3 * t + 1), at(3 * t + 2)];
      edge(a, b), edge(b, c), edge(c, a);
    }
    g.computeBoundingSphere();
    const size = Math.max(1e-6, g.boundingSphere?.radius ?? 1);
    for (let i = 0; i < n; i++) {
      const h = cnt[i] ? sum[i] / cnt[i] : 0;
      out[i] = Math.max(-1, Math.min(1, Math.sign(h) * Math.sqrt(Math.abs(h) * size * 0.1)));
    }
  }
  g.setAttribute("curvature", new THREE.BufferAttribute(out, 1));
  curvatureDone.add(g);
}

type Saved = {
  color: THREE.Color;
  emissive: THREE.Color;
  emissiveIntensity: number;
  transparent: boolean;
  opacity: number;
  depthWrite: boolean;
  colorWrite: boolean;
  onBeforeCompile: THREE.Material["onBeforeCompile"];
  customProgramCacheKey: THREE.Material["customProgramCacheKey"];
};

const EDGE_ANGLE = 30;
const edgeGeometries = new WeakMap<THREE.BufferGeometry, THREE.EdgesGeometry>();
const edgesOf = (g: THREE.BufferGeometry) => {
  let e = edgeGeometries.get(g);
  if (!e) edgeGeometries.set(g, (e = new THREE.EdgesGeometry(g, EDGE_ANGLE)));
  return e;
};

/** Smooth edges of a geometry (between faces bent less than EDGE_ANGLE): ends and the two faces' normals, for silhouettes. */
type Smooth = { data: Float32Array; count: number };
const smoothEdges = new WeakMap<THREE.BufferGeometry, Smooth>();
function smoothOf(g: THREE.BufferGeometry): Smooth {
  const known = smoothEdges.get(g);
  if (known) return known;
  const pos = g.getAttribute("position") as THREE.BufferAttribute;
  const index = g.getIndex();
  const n = index ? index.count : pos.count;
  const ids = new Map<string, number>();
  const weld = (i: number) => {
    const k = `${Math.round(pos.getX(i) * 1000)},${Math.round(pos.getY(i) * 1000)},${Math.round(pos.getZ(i) * 1000)}`;
    let id = ids.get(k);
    if (id === undefined) ids.set(k, (id = i));
    return id;
  };
  const corner = (k: number) => weld(index ? index.getX(k) : k);
  const a = new THREE.Vector3();
  const b = new THREE.Vector3();
  const c = new THREE.Vector3();
  const normals: THREE.Vector3[] = [];
  const edges = new Map<string, number[]>();
  for (let t = 0; t < n / 3; t++) {
    const v = [corner(3 * t), corner(3 * t + 1), corner(3 * t + 2)];
    a.fromBufferAttribute(pos, v[0]);
    b.fromBufferAttribute(pos, v[1]);
    c.fromBufferAttribute(pos, v[2]);
    normals.push(b.clone().sub(a).cross(c.clone().sub(a)).normalize());
    for (let e = 0; e < 3; e++) {
      const [p, q] = [v[e], v[(e + 1) % 3]];
      const key = p < q ? `${p}:${q}` : `${q}:${p}`;
      const list = edges.get(key);
      if (list) list.push(t);
      else edges.set(key, [t]);
    }
  }
  const cos = Math.cos(THREE.MathUtils.degToRad(EDGE_ANGLE));
  const out: number[] = [];
  for (const [key, ts] of edges) {
    if (ts.length !== 2) continue;
    const [n1, n2] = [normals[ts[0]], normals[ts[1]]];
    if (n1.dot(n2) < cos) continue; // a crease: drawn as a feature edge already
    const [p, q] = key.split(":").map(Number);
    out.push(pos.getX(p), pos.getY(p), pos.getZ(p), pos.getX(q), pos.getY(q), pos.getZ(q), n1.x, n1.y, n1.z, n2.x, n2.y, n2.z);
  }
  const smooth = { data: new Float32Array(out), count: out.length / 12 };
  smoothEdges.set(g, smooth);
  return smooth;
}

/** The outline of a curved body as seen from the camera: smooth edges between a face turned towards it and one turned away. */
function silhouette(line: THREE.LineSegments, body: THREE.Mesh, camera: THREE.Camera) {
  const smooth = smoothOf(body.geometry);
  if (!smooth.count || smooth.count > 300000) return;
  const key = camera.matrixWorld.elements.join(",") + body.matrixWorld.elements.join(",");
  if (line.userData.key === key) return;
  line.userData.key = key;
  const inverse = new THREE.Matrix4().copy(body.matrixWorld).invert();
  const ortho = (camera as THREE.OrthographicCamera).isOrthographicCamera === true;
  const eye = new THREE.Vector3().setFromMatrixPosition(camera.matrixWorld).applyMatrix4(inverse);
  // Towards the eye, for a parallel view.
  const back = new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld, 2).transformDirection(inverse);
  const geometry = line.geometry;
  let attr = geometry.getAttribute("position") as THREE.BufferAttribute | undefined;
  if (!attr || attr.count < smooth.count * 2) {
    attr = new THREE.BufferAttribute(new Float32Array(smooth.count * 6), 3);
    attr.setUsage(THREE.DynamicDrawUsage);
    geometry.setAttribute("position", attr);
  }
  const d = smooth.data;
  const out = attr.array as Float32Array;
  let k = 0;
  for (let i = 0; i < smooth.count; i++) {
    const o = i * 12;
    const tx = ortho ? back.x : eye.x - d[o];
    const ty = ortho ? back.y : eye.y - d[o + 1];
    const tz = ortho ? back.z : eye.z - d[o + 2];
    const f1 = d[o + 6] * tx + d[o + 7] * ty + d[o + 8] * tz;
    const f2 = d[o + 9] * tx + d[o + 10] * ty + d[o + 11] * tz;
    if (f1 > 0 === f2 > 0) continue;
    for (let j = 0; j < 6; j++) out[k++] = d[o + j];
  }
  attr.needsUpdate = true;
  geometry.setDrawRange(0, k / 3);
}

export class Display {
  mode: DisplayMode = "shaded";
  /** Clipping planes the edge lines are cut by too (a section). */
  clipping: THREE.Plane[] | null = null;
  private saved = new Map<THREE.MeshStandardMaterial, Saved>();
  private lines = new Map<THREE.Mesh, THREE.LineSegments>();
  private outlines = new Map<THREE.Mesh, THREE.LineSegments>();
  private lineMaterial = new THREE.LineBasicMaterial({ color: "#2a2f3a", toneMapped: false, fog: false });
  private xrayLineMaterial = new THREE.LineBasicMaterial({ color: "#3d4a63", transparent: true, opacity: 0.55, depthTest: false, toneMapped: false, fog: false });
  /** Fine detail (engraving, graduations) drawn as lines in the shaded view. */
  readonly detail = new DetailLines();

  /** Brings every body (and its edges) to the mode; call before each draw (`heightPx`: the view's height). */
  apply(bodies: THREE.Mesh[], materialsOf: (m: THREE.Mesh) => THREE.MeshStandardMaterial[], camera: THREE.Camera, heightPx = 800) {
    const mode = this.mode;
    this.detail.apply(mode === "shaded" ? bodies : [], materialsOf, camera, heightPx, this.clipping);
    const analysis = ANALYSIS.has(mode);
    const edges = mode !== "shaded" && !analysis;
    // Edge lines: one per body, sharing the body's geometry's edges.
    const seen = new Set<THREE.Mesh>();
    if (edges) {
      const material = mode === "xray" ? this.xrayLineMaterial : this.lineMaterial;
      material.clippingPlanes = this.clipping;
      for (const body of bodies) {
        seen.add(body);
        let line = this.lines.get(body);
        if (!line || line.parent !== body || line.userData.geometry !== body.geometry) {
          if (line) line.removeFromParent();
          line = new THREE.LineSegments(edgesOf(body.geometry), material);
          line.userData.geometry = body.geometry;
          line.raycast = () => {}; // never picked
          line.renderOrder = mode === "xray" ? 960 : 1;
          body.add(line);
          this.lines.set(body, line);
        }
        line.material = material;
        line.renderOrder = mode === "xray" ? 960 : 1;
      }
    }
    for (const [body, line] of this.lines) {
      if (seen.has(body)) continue;
      line.removeFromParent();
      this.lines.delete(body);
    }
    // Outlines of curved faces where no shading shows them.
    const outlined = new Set<THREE.Mesh>();
    if (mode === "wireframe" || mode === "hidden" || mode === "xray") {
      const material = mode === "xray" ? this.xrayLineMaterial : this.lineMaterial;
      for (const body of bodies) {
        let line = this.outlines.get(body);
        if (!line || line.parent !== body) {
          line?.removeFromParent();
          line = new THREE.LineSegments(new THREE.BufferGeometry(), material);
          line.raycast = () => {};
          line.frustumCulled = false;
          body.add(line);
          this.outlines.set(body, line);
        }
        if (line.userData.geometry !== body.geometry) {
          line.userData.geometry = body.geometry;
          line.userData.key = null;
        }
        line.material = material;
        line.renderOrder = mode === "xray" ? 960 : 1;
        silhouette(line, body, camera);
        outlined.add(body);
      }
    }
    for (const [body, line] of this.outlines) {
      if (outlined.has(body)) continue;
      line.removeFromParent();
      line.geometry.dispose();
      this.outlines.delete(body);
    }
    if (mode === "shaded" || mode === "edges") {
      this.restore();
      return;
    }
    if (mode === "curvature") for (const body of bodies) ensureCurvature(body.geometry);
    for (const body of bodies)
      for (const m of materialsOf(body)) {
        if (!this.saved.has(m))
          this.saved.set(m, {
            color: m.color.clone(),
            emissive: m.emissive.clone(),
            emissiveIntensity: m.emissiveIntensity,
            transparent: m.transparent,
            opacity: m.opacity,
            depthWrite: m.depthWrite,
            colorWrite: m.colorWrite,
            onBeforeCompile: m.onBeforeCompile,
            customProgramCacheKey: m.customProgramCacheKey,
          });
        let changed = false;
        const set = <K extends keyof THREE.MeshStandardMaterial>(k: K, v: THREE.MeshStandardMaterial[K]) => {
          if (m[k] !== v) {
            (m[k] as unknown) = v;
            changed = true;
          }
        };
        if (analysis) {
          set("colorWrite", true);
          set("depthWrite", true);
          set("transparent", false);
          set("opacity", 1);
          if (m.userData.analysis !== mode) {
            m.userData.analysis = mode;
            m.onBeforeCompile = analysisPatch(mode);
            m.customProgramCacheKey = () => `analysis-${mode}`;
            changed = true;
          }
        } else if (m.userData.analysis) {
          const saved = this.saved.get(m)!;
          m.onBeforeCompile = saved.onBeforeCompile;
          m.customProgramCacheKey = saved.customProgramCacheKey;
          delete m.userData.analysis;
          changed = true;
        }
        if (analysis) {
          // (the shader patch colours it)
        } else if (mode === "wireframe") {
          // Depth only where the faces are is not drawn: every edge shows, front and back.
          set("colorWrite", false);
          set("depthWrite", false);
          set("transparent", false);
        } else if (mode === "hidden") {
          // Faces drawn in the background colour: edges behind them are hidden.
          set("colorWrite", true);
          set("depthWrite", true);
          set("transparent", false);
          set("opacity", 1);
          if (!m.color.equals(WHITE)) m.color.copy(WHITE);
          m.emissive.copy(WHITE);
          m.emissiveIntensity = 1;
        } else {
          set("colorWrite", true);
          set("transparent", true);
          set("opacity", 0.32);
          set("depthWrite", false);
        }
        if (changed) m.needsUpdate = true;
      }
  }

  /** Puts back every material this changed. */
  restore() {
    for (const [m, s] of this.saved) {
      m.color.copy(s.color);
      m.emissive.copy(s.emissive);
      m.emissiveIntensity = s.emissiveIntensity;
      m.transparent = s.transparent;
      m.opacity = s.opacity;
      m.depthWrite = s.depthWrite;
      m.colorWrite = s.colorWrite;
      m.onBeforeCompile = s.onBeforeCompile;
      m.customProgramCacheKey = s.customProgramCacheKey;
      delete m.userData.analysis;
      m.needsUpdate = true;
    }
    this.saved.clear();
  }

  /** Bodies hidden in this mode's pass for shadows (wireframe and hidden line cast none). */
  get castsShadows() {
    return this.mode === "shaded" || this.mode === "edges" || ANALYSIS.has(this.mode);
  }
}

const WHITE = new THREE.Color("#ffffff");
