// The stock parts browser: real bought-in parts from the engine's stock
// library (ISO screws, nuts, washers, inserts, bearings, servos, steppers,
// extrusion…), each with its true geometry, part number, price, mass and
// where to buy it. Search by family, standard, size or part number, pick a
// size, insert it into the project or an assembly (`stock.insert`).

import { useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import { type EntityView, projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";
import { assemblyDef, isPart, materialLook, shapeOf } from "./model";
import "./StockBrowser.css";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;

type Source = { supplier: string; sku?: string; url?: string; unit_price?: number; currency?: string };
export type StockSize = {
  name: string;
  values: Record<string, number>;
  partNumber?: string;
  description?: string;
  manufacturer?: string;
  mpn?: string;
  unitPrice?: number;
  currency?: string;
  pack?: number;
  massG?: number;
  sources?: Source[];
  size?: [number, number, number];
  min?: [number, number, number];
  connectors?: string[];
  metrics?: Record<string, { value: number; unit: string }>;
  parts?: string[];
  error?: string;
};
export type StockFamily = {
  id: string;
  name: string;
  title: string;
  standard?: string;
  category: string;
  summary: string;
  keywords: string[];
  default: string;
  columns: { key: string; label: string; unit: string }[];
  materials: Record<string, number>;
  sizes: StockSize[];
};
type Catalogue = { categories: string[]; families: StockFamily[]; inProject: Record<string, string> };

type Preview = {
  positions: number[];
  normals: number[];
  indices: number[];
  bounds: [number[], number[]];
  /** The material of a program returning one solid. */
  material?: string | null;
  parts: { name: string; material: string | null; start: number; count: number }[];
};

// ---- Search ---------------------------------------------------------------------

const norm = (s: string) => s.toLowerCase().replace(/[×x*]/g, "x").replace(/\s+/g, " ").trim();

function familyText(f: StockFamily) {
  return norm([f.name, f.standard ?? "", f.category, f.summary, ...f.keywords].join(" "));
}
function sizeText(s: StockSize) {
  return norm([s.name, s.partNumber ?? "", s.mpn ?? "", s.manufacturer ?? "", ...(s.sources ?? []).map((x) => x.sku ?? "")].join(" "));
}

/** Families matching every word (in the family's own words or in a size's), with the sizes that matched. */
function search(families: StockFamily[], query: string, category: string | null) {
  const words = norm(query).split(" ").filter(Boolean);
  const out: { family: StockFamily; sizes: StockSize[] }[] = [];
  for (const f of families) {
    if (category && f.category !== category) continue;
    if (!words.length) {
      out.push({ family: f, sizes: [] });
      continue;
    }
    const ft = familyText(f);
    const rest = words.filter((w) => !ft.includes(w));
    if (!rest.length) {
      out.push({ family: f, sizes: [] });
      continue;
    }
    const sizes = f.sizes.filter((s) => {
      const st = sizeText(s);
      return rest.every((w) => st.includes(w));
    });
    if (sizes.length) out.push({ family: f, sizes });
  }
  return out;
}

// ---- Rendering ---------------------------------------------------------------------

function geometryOf(p: Preview) {
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.Float32BufferAttribute(p.positions, 3));
  if (p.normals.length === p.positions.length) g.setAttribute("normal", new THREE.Float32BufferAttribute(p.normals, 3));
  g.setIndex(p.indices);
  if (p.normals.length !== p.positions.length) g.computeVertexNormals();
  const materials: THREE.Material[] = [];
  const looks = new Map<string, number>();
  const look = (name: string | null) => {
    const key = name ?? "";
    let i = looks.get(key);
    if (i === undefined) {
      const l = materialLook(name ?? undefined);
      i = materials.length;
      materials.push(new THREE.MeshStandardMaterial({ color: l.color, metalness: l.metalness, roughness: l.roughness, transparent: (l.opacity ?? 1) < 1, opacity: l.opacity ?? 1 }));
      looks.set(key, i);
    }
    return i;
  };
  if (p.parts.length) for (const part of p.parts) g.addGroup(part.start, part.count, look(part.material));
  else g.addGroup(0, p.indices.length, look(p.material ?? null));
  return { geometry: g, materials };
}

function lights(scene: THREE.Scene, renderer: THREE.WebGLRenderer) {
  const pmrem = new THREE.PMREMGenerator(renderer);
  scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
  pmrem.dispose();
  const hemi = new THREE.HemisphereLight(0xeef4ff, 0x5a6470, 0.55);
  hemi.position.set(0, 0, 1);
  const key = new THREE.DirectionalLight(0xfff3e2, 1.7);
  key.position.set(1, -1.4, 1.8);
  const rim = new THREE.DirectionalLight(0xcfdcff, 1.2);
  rim.position.set(-1.2, 1.4, 0.6);
  scene.add(hemi, key, rim);
}

/** Puts the camera on a three-quarter view that frames the object. */
function frame(camera: THREE.PerspectiveCamera, box: THREE.Box3, zoom = 1) {
  const sphere = box.getBoundingSphere(new THREE.Sphere());
  const dist = (sphere.radius / Math.sin((camera.fov * Math.PI) / 360)) * 1.08 * zoom;
  const dir = new THREE.Vector3(0.95, -1.25, 0.85).normalize();
  camera.position.copy(sphere.center).addScaledVector(dir, dist);
  camera.near = dist / 100;
  camera.far = dist * 10;
  camera.up.set(0, 0, 1);
  camera.lookAt(sphere.center);
  camera.updateProjectionMatrix();
  return sphere.center;
}

const previews = new Map<string, Promise<Preview>>();
function loadPreview(family: string, size: string): Promise<Preview> {
  const key = `${family}\n${size}`;
  let p = previews.get(key);
  if (!p) {
    p = fetch(`/stock/${encodeURIComponent(family)}/preview?size=${encodeURIComponent(size)}`).then(async (r) => {
      if (!r.ok) throw new Error(await r.text());
      return r.json() as Promise<Preview>;
    });
    p.catch(() => previews.delete(key));
    previews.set(key, p);
  }
  return p;
}

// One offscreen renderer draws every thumbnail, one at a time.
let thumbRenderer: { renderer: THREE.WebGLRenderer; scene: THREE.Scene; camera: THREE.PerspectiveCamera } | null = null;
const thumbs = new Map<string, Promise<string>>();
let queue: Promise<unknown> = Promise.resolve();
const THUMB_W = 360;
const THUMB_H = 270;

function thumbnail(family: string, size: string): Promise<string> {
  const key = `${family}\n${size}`;
  let t = thumbs.get(key);
  if (t) return t;
  t = loadPreview(family, size).then(
    (p) =>
      (queue = queue.then(() => {
        if (!thumbRenderer) {
          const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: true });
          renderer.setPixelRatio(2);
          renderer.setSize(THUMB_W, THUMB_H, false);
          renderer.setClearColor(0x000000, 0);
          renderer.toneMapping = THREE.ACESFilmicToneMapping;
          renderer.toneMappingExposure = 1.0;
          const scene = new THREE.Scene();
          lights(scene, renderer);
          thumbRenderer = { renderer, scene, camera: new THREE.PerspectiveCamera(28, THUMB_W / THUMB_H, 0.1, 10000) };
        }
        const { renderer, scene, camera } = thumbRenderer;
        const { geometry, materials } = geometryOf(p);
        const mesh = new THREE.Mesh(geometry, materials);
        scene.add(mesh);
        geometry.computeBoundingBox();
        frame(camera, geometry.boundingBox!, 0.92);
        renderer.render(scene, camera);
        const url = renderer.domElement.toDataURL("image/png");
        scene.remove(mesh);
        geometry.dispose();
        materials.forEach((m) => m.dispose());
        return url;
      })) as Promise<string>,
  );
  t.catch(() => thumbs.delete(key));
  thumbs.set(key, t);
  return t;
}

function Thumb(props: { family: string; size: string; alt: string }) {
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let live = true;
    setUrl(null);
    setFailed(false);
    thumbnail(props.family, props.size).then(
      (u) => live && setUrl(u),
      () => live && setFailed(true),
    );
    return () => {
      live = false;
    };
  }, [props.family, props.size]);
  if (url) return <img className="stock-thumb" src={url} alt={props.alt} draggable={false} />;
  return <div className={`stock-thumb placeholder ${failed ? "failed" : ""}`}>{failed ? <Icon name="alert" /> : <span className="stock-spinner" />}</div>;
}

/** The selected size, live: turn it with the mouse; it turns slowly by itself until touched. */
function LivePreview(props: { family: string; size: string }) {
  const host = useRef<HTMLDivElement>(null);
  const three = useRef<{ renderer: THREE.WebGLRenderer; scene: THREE.Scene; camera: THREE.PerspectiveCamera; controls: OrbitControls; mesh: THREE.Mesh | null; frame: number } | null>(null);
  const [state, setState] = useState<"loading" | "ready" | "failed">("loading");

  useEffect(() => {
    const el = host.current!;
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setClearColor(0x000000, 0);
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1.0;
    el.appendChild(renderer.domElement);
    const scene = new THREE.Scene();
    lights(scene, renderer);
    const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 10000);
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.autoRotate = true;
    controls.autoRotateSpeed = 1.6;
    controls.enablePan = false;
    const stop = () => (controls.autoRotate = false);
    renderer.domElement.addEventListener("pointerdown", stop);
    const t = { renderer, scene, camera, controls, mesh: null as THREE.Mesh | null, frame: 0 };
    three.current = t;
    const resize = () => {
      const w = el.clientWidth || 1;
      const h = el.clientHeight || 1;
      renderer.setSize(w, h, false);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    };
    const ro = new ResizeObserver(resize);
    ro.observe(el);
    resize();
    const tick = () => {
      t.frame = requestAnimationFrame(tick);
      controls.update();
      renderer.render(scene, camera);
    };
    tick();
    return () => {
      cancelAnimationFrame(t.frame);
      ro.disconnect();
      controls.dispose();
      renderer.domElement.removeEventListener("pointerdown", stop);
      if (t.mesh) {
        t.mesh.geometry.dispose();
        (t.mesh.material as THREE.Material[]).forEach((m) => m.dispose());
      }
      scene.environment?.dispose();
      renderer.dispose();
      renderer.domElement.remove();
      three.current = null;
    };
  }, []);

  useEffect(() => {
    let live = true;
    setState("loading");
    loadPreview(props.family, props.size).then(
      (p) => {
        const t = three.current;
        if (!live || !t) return;
        if (t.mesh) {
          t.scene.remove(t.mesh);
          t.mesh.geometry.dispose();
          (t.mesh.material as THREE.Material[]).forEach((m) => m.dispose());
        }
        const { geometry, materials } = geometryOf(p);
        t.mesh = new THREE.Mesh(geometry, materials);
        t.scene.add(t.mesh);
        geometry.computeBoundingBox();
        const centre = frame(t.camera, geometry.boundingBox!, 1.0);
        t.controls.target.copy(centre);
        t.controls.update();
        setState("ready");
      },
      () => live && setState("failed"),
    );
    return () => {
      live = false;
    };
  }, [props.family, props.size]);

  return (
    <div className={`stock-live ${state}`} ref={host}>
      {state === "loading" && <span className="stock-spinner" />}
      {state === "failed" && <span className="stock-live-note">No preview for this size</span>}
    </div>
  );
}

// ---- Formatting ---------------------------------------------------------------------

const fmtNum = (v: number, digits = 2) => String(+v.toFixed(digits));
function money(v: number | undefined, currency = "USD") {
  if (v == null) return "—";
  const digits = v < 1 ? 3 : 2;
  try {
    return new Intl.NumberFormat("en-US", { style: "currency", currency, minimumFractionDigits: 2, maximumFractionDigits: digits }).format(v);
  } catch {
    return `${v.toFixed(digits)} ${currency}`;
  }
}
function grams(g: number | undefined) {
  if (g == null) return "—";
  if (g >= 1000) return `${fmtNum(g / 1000, 3)} kg`;
  if (g >= 10) return `${fmtNum(g, 1)} g`;
  return `${fmtNum(g, 3)} g`;
}

/** Sizes named “A×B” group by A (a screw's M3, an extrusion's 2020); others stand alone. */
function groupsOf(sizes: StockSize[]) {
  if (sizes.length > 8 && sizes.every((s) => s.name.includes("×"))) {
    const groups: { key: string; sizes: StockSize[] }[] = [];
    for (const s of sizes) {
      const key = s.name.slice(0, s.name.indexOf("×"));
      let g = groups.find((x) => x.key === key);
      if (!g) groups.push((g = { key, sizes: [] }));
      g.sizes.push(s);
    }
    if (groups.length > 1) return groups;
  }
  return null;
}

function rangeLabel(f: StockFamily) {
  const groups = groupsOf(f.sizes);
  if (groups) return `${groups[0].key}–${groups[groups.length - 1].key} · ${f.sizes.length} sizes`;
  if (f.sizes.length <= 3) return f.sizes.map((s) => s.name).join(" · ");
  return `${f.sizes[0].name} – ${f.sizes[f.sizes.length - 1].name} · ${f.sizes.length} sizes`;
}

// ---- The browser ----------------------------------------------------------------------

export function StockBrowser(props: {
  entities: ReadonlyMap<string, EntityView>;
  run: Run;
  notify: (text: string) => void;
  onInserted: (id: string) => void;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const searchBox = useRef<HTMLInputElement>(null);
  const [catalogue, setCatalogue] = useState<Catalogue | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [familyId, setFamilyId] = useState<string | null>(null);
  const [sizeName, setSizeName] = useState<string | null>(null);
  const [group, setGroup] = useState<string | null>(null);
  const [parent, setParent] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    const d = dialog.current!;
    if (!d.open) d.showModal();
    searchBox.current?.focus();
    return () => d.close();
  }, []);

  const load = () =>
    fetch(projectUrl("/stock"))
      .then(async (r) => {
        if (!r.ok) throw new Error(await r.text());
        return r.json() as Promise<Catalogue>;
      })
      .then(setCatalogue, (e: Error) => setError(e.message));
  useEffect(() => {
    void load();
  }, []);

  const results = useMemo(() => (catalogue ? search(catalogue.families, query, category) : []), [catalogue, query, category]);
  const counts = useMemo(() => {
    const c = new Map<string, number>();
    if (catalogue) for (const r of search(catalogue.families, query, null)) c.set(r.family.category, (c.get(r.family.category) ?? 0) + 1);
    return c;
  }, [catalogue, query]);

  const family = catalogue?.families.find((f) => f.id === familyId) ?? null;
  const matched = results.find((r) => r.family.id === familyId)?.sizes ?? [];
  const size = family?.sizes.find((s) => s.name === sizeName) ?? null;
  const groups = family ? groupsOf(family.sizes) : null;

  // Choosing a family picks a size: the one the search matched, else its default.
  const choose = (f: StockFamily, preferred?: StockSize) => {
    setFamilyId(f.id);
    const s = preferred ?? f.sizes.find((x) => x.name === f.default) ?? f.sizes[0];
    setSizeName(s.name);
    setGroup(s.name.includes("×") ? s.name.slice(0, s.name.indexOf("×")) : null);
  };
  // Keep the detail in step with the search: the first result opens.
  useEffect(() => {
    if (!catalogue) return;
    const first = results[0];
    if (!first) return;
    if (!family || !results.some((r) => r.family.id === family.id)) choose(first.family, first.sizes[0]);
    else if (matched.length && !matched.some((s) => s.name === sizeName)) choose(family, matched[0]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [catalogue, results]);

  const assemblies = useMemo(
    () => [...props.entities.values()].filter((e) => assemblyDef(e)).map((e) => ({ id: e.id, name: assemblyDef(e)!.name })),
    [props.entities],
  );

  const insert = async () => {
    if (!family || !size || busy) return;
    setBusy(true);
    // Beside what is already placed, standing on the ground.
    let at: [number, number, number] = [0, 0, 0];
    if (!parent) {
      const placed = [...props.entities.values()].filter(isPart).map(shapeOf);
      const right = placed.length ? Math.max(...placed.map((s) => s.x + s.width)) : null;
      const w = size.size?.[0] ?? 0;
      const min = size.min ?? [-w / 2, 0, 0];
      at = [right == null ? 0 : right + Math.max(10, w * 0.6) - min[0], 0, -min[2]];
      at = at.map((v) => Math.round(v * 100) / 100) as [number, number, number];
    }
    const out = (await props.run("stock.insert", { family: family.id, size: size.name, parent: parent || null, at })) as { output: { id: string; created: boolean } } | null;
    setBusy(false);
    if (out?.output?.id) {
      props.notify(`Inserted ${size.partNumber ?? `${family.name} ${size.name}`}${out.output.created ? ` · ${family.title} added with all ${family.sizes.length} sizes` : ""}`);
      props.onInserted(out.output.id);
      void load();
    }
  };

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      /* clipboard refused: nothing to do */
    }
  };

  const total = catalogue?.families.reduce((n, f) => n + f.sizes.length, 0) ?? 0;

  return (
    <dialog ref={dialog} className="stock-browser" onClose={props.onClose} onCancel={props.onClose} aria-label="Stock parts">
      <header className="stock-head">
        <div className="stock-title">
          <span className="stock-badge">
            <Icon name="package" />
          </span>
          <div>
            <h2>Stock parts</h2>
            <p>{catalogue ? `${catalogue.families.length} families · ${total.toLocaleString("en-US")} catalogue sizes · true geometry, part numbers and suppliers` : "Real bought-in parts with their true geometry"}</p>
          </div>
        </div>
        <label className="stock-search">
          <Icon name="search" />
          <input
            ref={searchBox}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search: M3×12, 608, ISO 7380, servo, 2040…"
            aria-label="Search stock parts"
            onKeyDown={(e) => {
              if (e.key === "Enter" && family && size) void insert();
            }}
          />
          {query && (
            <button type="button" className="stock-clear" aria-label="Clear search" onClick={() => setQuery("")}>
              <Icon name="close" />
            </button>
          )}
        </label>
        <IconButton icon="close" label="Close stock parts" onClick={props.onClose} />
      </header>

      <div className="stock-body">
        <nav className="stock-cats" aria-label="Categories">
          <button type="button" className={category === null ? "active" : ""} onClick={() => setCategory(null)}>
            <span>All parts</span>
            <small>{[...counts.values()].reduce((a, b) => a + b, 0) || ""}</small>
          </button>
          {catalogue?.categories
            .filter((c) => catalogue.families.some((f) => f.category === c))
            .map((c) => (
              <button type="button" key={c} className={category === c ? "active" : ""} onClick={() => setCategory(category === c ? null : c)} disabled={!counts.get(c)}>
                <span>{c}</span>
                <small>{counts.get(c) ?? 0}</small>
              </button>
            ))}
          <p className="stock-rule">Every bought item in a product comes from here or is modelled as true geometry with its stock data — never a stand-in block.</p>
        </nav>

        <section className="stock-grid" aria-label="Families">
          {error && <div className="stock-empty">The stock library did not load: {error}</div>}
          {!catalogue && !error && (
            <div className="stock-cards">
              {Array.from({ length: 8 }, (_, i) => (
                <div key={i} className="stock-card skeleton" />
              ))}
            </div>
          )}
          {catalogue && !results.length && (
            <div className="stock-empty">
              <strong>Nothing matches “{query}”.</strong>
              <span>Try a size (M4×20, 6002), a standard (ISO 4032) or a name (washer, stepper).</span>
            </div>
          )}
          <div className="stock-cards">
            {results.map(({ family: f, sizes }) => (
              <button
                type="button"
                key={f.id}
                className={`stock-card ${familyId === f.id ? "active" : ""}`}
                onClick={() => choose(f, sizes[0])}
                onDoubleClick={() => void insert()}
                aria-pressed={familyId === f.id}
              >
                <Thumb family={f.id} size={f.default} alt={f.title} />
                <span className="stock-card-text">
                  <strong>{f.name}</strong>
                  <span className="stock-card-std">{f.standard ?? f.category}</span>
                  <small>{sizes.length ? `${sizes.length} matching: ${sizes.slice(0, 3).map((s) => s.name).join(", ")}${sizes.length > 3 ? "…" : ""}` : rangeLabel(f)}</small>
                </span>
                {catalogue?.inProject[f.id] && <span className="stock-held" title="Already in this project: inserting reuses it">In project</span>}
              </button>
            ))}
          </div>
        </section>

        <aside className="stock-detail" aria-label="Selected part">
          {family && size ? (
            <>
              <div className="stock-detail-scroll">
              <LivePreview family={family.id} size={size.name} />
              <div className="stock-detail-head">
                <span className="stock-std">{family.standard ?? family.category}</span>
                <h3>
                  {family.name} <em>{size.name}</em>
                </h3>
                <p>{size.description ?? family.summary}</p>
              </div>

              <div className="stock-sizes">
                <div className="stock-label">
                  Size <small>{family.sizes.length} in the catalogue</small>
                </div>
                {groups ? (
                  <>
                    <div className="stock-chips major" role="tablist">
                      {groups.map((g) => (
                        <button
                          type="button"
                          role="tab"
                          key={g.key}
                          aria-selected={group === g.key}
                          className={group === g.key ? "active" : ""}
                          onClick={() => {
                            setGroup(g.key);
                            const same = g.sizes.find((s) => s.name.slice(s.name.indexOf("×")) === size.name.slice(size.name.indexOf("×")));
                            setSizeName((same ?? g.sizes[Math.floor(g.sizes.length / 3)]).name);
                          }}
                        >
                          {g.key}
                        </button>
                      ))}
                    </div>
                    <div className="stock-chips">
                      {(groups.find((g) => g.key === group) ?? groups[0]).sizes.map((s) => (
                        <button type="button" key={s.name} className={`${s.name === size.name ? "active" : ""} ${matched.some((m) => m.name === s.name) ? "hit" : ""}`} onClick={() => setSizeName(s.name)} title={s.partNumber}>
                          {s.name.slice(s.name.indexOf("×") + 1)}
                        </button>
                      ))}
                    </div>
                  </>
                ) : (
                  <div className="stock-chips">
                    {family.sizes.map((s) => (
                      <button type="button" key={s.name} className={`${s.name === size.name ? "active" : ""} ${matched.some((m) => m.name === s.name) ? "hit" : ""}`} onClick={() => setSizeName(s.name)} title={s.partNumber}>
                        {s.name}
                      </button>
                    ))}
                  </div>
                )}
              </div>

              {size.error ? (
                <div className="stock-empty">This size does not evaluate: {size.error}</div>
              ) : (
                <dl className="stock-facts">
                  <div className="wide">
                    <dt>Part number</dt>
                    <dd>
                      <code>{size.partNumber}</code>
                      {size.partNumber && (
                        <button type="button" className="stock-copy" onClick={() => void copy(size.partNumber!)} aria-label="Copy part number">
                          <Icon name={copied ? "check" : "copy"} />
                        </button>
                      )}
                    </dd>
                  </div>
                  {size.manufacturer && (
                    <div className="wide">
                      <dt>Maker</dt>
                      <dd>
                        {size.manufacturer}
                        {size.mpn ? ` · ${size.mpn}` : ""}
                      </dd>
                    </div>
                  )}
                  <div>
                    <dt>Typical price</dt>
                    <dd>
                      {money(size.unitPrice, size.currency)}
                      {size.pack && size.pack > 1 ? <small> each · packs of {size.pack}</small> : <small> each</small>}
                    </dd>
                  </div>
                  <div>
                    <dt>Mass</dt>
                    <dd>{grams(size.massG)}</dd>
                  </div>
                  {size.size && (
                    <div>
                      <dt>Overall</dt>
                      <dd>{size.size.map((v) => fmtNum(v, 1)).join(" × ")} mm</dd>
                    </div>
                  )}
                  {Object.entries(size.metrics ?? {})
                    .slice(0, 5)
                    .map(([k, m]) => (
                      <div key={k}>
                        <dt>{k}</dt>
                        <dd>
                          {fmtNum(m.value)} {m.unit}
                        </dd>
                      </div>
                    ))}
                  {!!size.connectors?.length && (
                    <div className="wide">
                      <dt>Connectors</dt>
                      <dd className="stock-connectors">
                        {size.connectors.map((c) => (
                          <span key={c}>{c}</span>
                        ))}
                      </dd>
                    </div>
                  )}
                  {!!size.sources?.length && (
                    <div className="wide">
                      <dt>Buy from</dt>
                      <dd className="stock-sources">
                        {size.sources.map((s, i) => (
                          <a key={i} href={s.url} target="_blank" rel="noreferrer noopener">
                            <strong>{s.supplier}</strong>
                            <span>{s.sku ?? "search"}</span>
                            <Icon name="external" />
                          </a>
                        ))}
                      </dd>
                    </div>
                  )}
                </dl>
              )}

              </div>
              <footer className="stock-insert">
                <label>
                  <span>Into</span>
                  <select value={parent} onChange={(e) => setParent(e.target.value)}>
                    <option value="">The project</option>
                    {assemblies.map((a) => (
                      <option key={a.id} value={a.id}>
                        {a.name}
                      </option>
                    ))}
                  </select>
                </label>
                <button type="button" className="primary" disabled={busy || !!size.error} onClick={() => void insert()}>
                  {busy ? <span className="stock-spinner small" /> : <Icon name="plus" />}
                  Insert {size.name}
                </button>
              </footer>
            </>
          ) : (
            <div className="stock-empty">Pick a part to see its sizes.</div>
          )}
        </aside>
      </div>
    </dialog>
  );
}
