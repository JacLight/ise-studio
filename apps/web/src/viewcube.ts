// The view cube: a small cube in the corner that turns with the view. Drag it
// to turn the view; the arrows round it turn the view 90° (to reach the back
// and the bottom in one click); its menu lists every standard view. Each
// face is split 3×3 like the cubes in other CAD tools: the middle of a face
// looks straight at it, the strips along its sides look at that edge (two
// faces at 45°) and the corners at that corner (three faces, isometric).
// Drawn with CSS 3D (no second WebGL context); faces turned away are hidden
// in script rather than with backface-visibility, which a mirrored matrix
// (screen Y points down) confuses.

import * as THREE from "three";

type V = [number, number, number];

/** Each face: outward normal, then its right and down directions as seen looking at it. */
const FACES: { name: string; n: V; u: V; d: V }[] = [
  { name: "Front", n: [0, -1, 0], u: [1, 0, 0], d: [0, 0, -1] },
  { name: "Back", n: [0, 1, 0], u: [-1, 0, 0], d: [0, 0, -1] },
  { name: "Right", n: [1, 0, 0], u: [0, 1, 0], d: [0, 0, -1] },
  { name: "Left", n: [-1, 0, 0], u: [0, -1, 0], d: [0, 0, -1] },
  { name: "Top", n: [0, 0, 1], u: [1, 0, 0], d: [0, -1, 0] },
  { name: "Bottom", n: [0, 0, -1], u: [-1, 0, 0], d: [0, -1, 0] },
];

const SIZE = 56;
/** Pixels a press on the cube may move and still be a click. */
const SLOP = 3;

export type CubeStep = "left" | "right" | "up" | "down";
const VIEWS: [string, string][] = [
  ["front", "Front"],
  ["back", "Back"],
  ["left", "Left"],
  ["right", "Right"],
  ["top", "Top"],
  ["bottom", "Bottom"],
  ["iso", "Isometric"],
];

export type CubeHandlers = {
  pick: (direction: THREE.Vector3) => void;
  home: () => void;
  /** A drag on the cube, in pixels. */
  drag: (dx: number, dy: number) => void;
  step: (step: CubeStep) => void;
  view: (name: string) => void;
};

/** The view direction a cell of a face asks for (cell i, j in −1..1 across and down). */
export function cellDirection(n: V, u: V, d: V, i: number, j: number): V {
  const v: V = [n[0] + i * u[0] + j * d[0], n[1] + i * u[1] + j * d[1], n[2] + i * u[2] + j * d[2]];
  const len = Math.hypot(...v);
  return [v[0] / len, v[1] / len, v[2] / len];
}

export class ViewCube {
  readonly root: HTMLDivElement;
  private cube: HTMLDivElement;
  private faces: { el: HTMLDivElement; n: THREE.Vector3 }[] = [];

  constructor(on: CubeHandlers) {
    this.root = document.createElement("div");
    this.root.className = "view-cube";
    this.root.setAttribute("role", "group");
    this.root.setAttribute("aria-label", "View cube: click a face, edge or corner to look from there; drag to turn the view");
    // Drag to turn; a press that barely moves stays a click on a face.
    let press: { id: number; x: number; y: number; moved: boolean } | null = null;
    this.root.addEventListener("pointerdown", (e) => {
      if (e.button !== 0 || !(e.target as HTMLElement).closest(".view-cube-body")) return;
      press = { id: e.pointerId, x: e.clientX, y: e.clientY, moved: false };
      e.stopPropagation();
    });
    this.root.addEventListener("pointermove", (e) => {
      if (!press || e.pointerId !== press.id) return;
      const dx = e.clientX - press.x;
      const dy = e.clientY - press.y;
      if (!press.moved && Math.hypot(dx, dy) < SLOP) return;
      if (!press.moved) {
        press.moved = true;
        this.root.setPointerCapture(e.pointerId);
        this.root.classList.add("is-dragging");
      }
      on.drag(dx, dy);
      press.x = e.clientX;
      press.y = e.clientY;
    });
    const release = (e: PointerEvent) => {
      if (!press || e.pointerId !== press.id) return;
      if (press.moved) {
        this.root.classList.remove("is-dragging");
        // Swallow the click that ends a drag.
        const stop = (c: Event) => c.stopPropagation();
        this.root.addEventListener("click", stop, { capture: true, once: true });
        setTimeout(() => this.root.removeEventListener("click", stop, { capture: true }), 0);
      }
      press = null;
    };
    this.root.addEventListener("pointerup", release);
    this.root.addEventListener("pointercancel", release);
    this.cube = document.createElement("div");
    this.cube.className = "view-cube-body";
    this.root.append(this.cube);
    const h = SIZE / 2;
    for (const f of FACES) {
      const el = document.createElement("div");
      el.className = "view-cube-face";
      const m = [...f.u, 0, ...f.d, 0, ...f.n, 0, f.n[0] * h, f.n[1] * h, f.n[2] * h, 1];
      el.style.transform = `matrix3d(${m.join(",")})`;
      const label = document.createElement("span");
      label.textContent = f.name;
      el.append(label);
      for (let j = -1; j <= 1; j++)
        for (let i = -1; i <= 1; i++) {
          const cell = document.createElement("button");
          cell.type = "button";
          cell.tabIndex = -1;
          cell.className = `view-cube-cell ${i === 0 && j === 0 ? "is-face" : i === 0 || j === 0 ? "is-edge" : "is-corner"}`;
          const dir = cellDirection(f.n, f.u, f.d, i, j);
          cell.title = i === 0 && j === 0 ? `${f.name} view` : i === 0 || j === 0 ? "Edge view" : "Corner view";
          cell.addEventListener("click", (e) => {
            e.stopPropagation();
            on.pick(new THREE.Vector3(...dir));
          });
          el.append(cell);
        }
      this.cube.append(el);
      this.faces.push({ el, n: new THREE.Vector3(...f.n) });
    }
    const home = document.createElement("button");
    home.type = "button";
    home.className = "view-cube-home";
    home.title = "Home view (fit all)";
    home.setAttribute("aria-label", "Home view");
    home.innerHTML = `<svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="M2 7.5 8 2.5l6 5M4 6.5V13h3v-3.5h2V13h3V6.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/></svg>`;
    home.addEventListener("click", (e) => {
      e.stopPropagation();
      on.home();
    });
    this.root.append(home);

    const arrows: [CubeStep, string, string][] = [
      ["left", "Turn the view 90° left", "M10 3.5 5.5 8l4.5 4.5"],
      ["right", "Turn the view 90° right", "M6 3.5 10.5 8 6 12.5"],
      ["up", "Turn the view 90° up", "M3.5 10 8 5.5l4.5 4.5"],
      ["down", "Turn the view 90° down", "M3.5 6 8 10.5 12.5 6"],
    ];
    for (const [step, title, d] of arrows) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = `view-cube-arrow is-${step}`;
      b.title = title;
      b.setAttribute("aria-label", title);
      b.innerHTML = `<svg viewBox="0 0 16 16" width="10" height="10" aria-hidden="true"><path d="${d}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
      b.addEventListener("click", (e) => {
        e.stopPropagation();
        on.step(step);
      });
      this.root.append(b);
    }

    // Every standard view, including the ones facing away.
    const more = document.createElement("button");
    more.type = "button";
    more.className = "view-cube-more";
    more.title = "All views";
    more.setAttribute("aria-label", "All views");
    more.setAttribute("aria-haspopup", "menu");
    more.innerHTML = `<svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><path d="M4 6.5 8 10l4-3.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
    const menu = document.createElement("div");
    menu.className = "view-cube-menu";
    menu.setAttribute("role", "menu");
    menu.hidden = true;
    const keys: Record<string, string> = { front: "1", back: "4", left: "6", right: "3", top: "2", bottom: "5", iso: "0" };
    for (const [name, label] of VIEWS) {
      const item = document.createElement("button");
      item.type = "button";
      item.setAttribute("role", "menuitem");
      item.innerHTML = `<span>${label}</span><kbd>${keys[name]}</kbd>`;
      item.addEventListener("click", (e) => {
        e.stopPropagation();
        menu.hidden = true;
        more.setAttribute("aria-expanded", "false");
        on.view(name);
      });
      menu.append(item);
    }
    const close = (e: Event) => {
      if (!menu.hidden && !menu.contains(e.target as Node) && e.target !== more && !more.contains(e.target as Node)) {
        menu.hidden = true;
        more.setAttribute("aria-expanded", "false");
      }
    };
    document.addEventListener("pointerdown", close, true);
    more.addEventListener("click", (e) => {
      e.stopPropagation();
      menu.hidden = !menu.hidden;
      more.setAttribute("aria-expanded", String(!menu.hidden));
    });
    this.root.append(more, menu);
  }

  /** Turns the cube to match the camera. */
  update(camera: THREE.Camera) {
    const e = camera.matrixWorldInverse.elements;
    // Rows of the view rotation: right, up, back (column-major elements).
    const r = [e[0], e[4], e[8]];
    const u = [e[1], e[5], e[9]];
    const b = [e[2], e[6], e[10]];
    this.cube.style.transform = `matrix3d(${[r[0], -u[0], b[0], 0, r[1], -u[1], b[1], 0, r[2], -u[2], b[2], 0, 0, 0, 0, 1].join(",")})`;
    const back = new THREE.Vector3(b[0], b[1], b[2]);
    for (const f of this.faces) {
      const facing = f.n.dot(back);
      f.el.classList.toggle("is-away", facing < 0.02);
      f.el.style.setProperty("--facing", Math.max(0, facing).toFixed(3));
    }
  }
}
