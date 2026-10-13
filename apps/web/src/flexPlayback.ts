// Shapes that change while an assembly plays: a spring stretched by the arm
// it pulls on, a servo's output spline turning with its joint. A played
// motion (a joint's sweep, a motion study, a robot path) lists them with
// their parameter values at every frame (`flex`); here a few frames are
// evaluated by the engine once (cached by shape and values), and the view
// blends between the two nearest while it plays, so the spring stretches
// smoothly at full frame rate however fast the motion runs.

import type { MeshData, ShapeStats } from "./engine";

/** A shape whose parameters follow the joints, with its values at each frame. */
export type FlexShape = {
  /** The swept assembly's instance it is in. */
  instance: string;
  /** Instances from that one down to the shape. */
  path: string[];
  shape: string;
  config?: string | null;
  values: Record<string, number>[];
};

/** What a flexing shape shows at one moment: a mesh, or two blended. */
export type FlexMesh = { a: MeshData; b: MeshData | null; t: number; parts: { index_start: number; index_count: number }[] };

/** Flexing shapes at a moment, by `${path from the flex instance down}|${shape}`. */
export type FlexPose = Map<string, FlexMesh>;

export const flexKey = (path: string[], shape: string) => `${path.join("/")}|${shape}`;

type Preview = (shape: string, values: Record<string, number>, config?: string) => Promise<{ stats: ShapeStats; mesh: MeshData }>;

/** Frames evaluated per flexing shape (the rest are blended). */
const SAMPLES = 24;
/** Evaluated meshes kept (oldest dropped first). */
const KEEP = 400;

export class FlexSamples {
  private meshes = new Map<string, { mesh: MeshData; parts: FlexMesh["parts"] }>();
  private loading = new Map<string, Promise<void>>();
  private queue: { key: string; run: () => Promise<void> }[] = [];
  private busy = false;
  /** Called when a sample arrives (the view redraws with it). */
  onLoad: () => void = () => {};

  constructor(private preview: Preview) {}

  private sampleKey(f: FlexShape, values: Record<string, number>) {
    return `${f.shape}|${f.config ?? ""}|${JSON.stringify(values)}`;
  }

  /** The frames of a shape that are evaluated: evenly through the motion, ends included. */
  private samplesOf(f: FlexShape): number[] {
    const n = f.values.length;
    if (n <= SAMPLES) return [...Array(n).keys()];
    const out: number[] = [];
    for (let j = 0; j < SAMPLES; j++) out.push(Math.round((j * (n - 1)) / (SAMPLES - 1)));
    return [...new Set(out)];
  }

  private load(f: FlexShape, frame: number, urgent: boolean): Promise<void> {
    const values = f.values[frame];
    const key = this.sampleKey(f, values);
    if (this.meshes.has(key)) return Promise.resolve();
    const known = this.loading.get(key);
    if (known) {
      // Wanted now: move it to the front of the line.
      if (urgent) {
        const i = this.queue.findIndex((q) => q.key === key);
        if (i > 0) this.queue.unshift(...this.queue.splice(i, 1));
      }
      return known;
    }
    let done!: () => void;
    const p = new Promise<void>((r) => (done = r));
    this.loading.set(key, p);
    const run = async () => {
      try {
        const r = await this.preview(f.shape, values, f.config ?? undefined);
        this.meshes.set(key, { mesh: r.mesh, parts: r.stats.parts ?? [] });
        if (this.meshes.size > KEEP) this.meshes.delete(this.meshes.keys().next().value!);
      } catch {
        // A step the shape cannot make shows the nearest one that it can.
      }
      this.loading.delete(key);
      done();
      this.onLoad();
    };
    if (urgent) this.queue.unshift({ key, run });
    else this.queue.push({ key, run });
    void this.pump();
    return p;
  }

  /** One evaluation at a time: the engine answers the person's own edits in between. */
  private async pump() {
    if (this.busy) return;
    this.busy = true;
    while (this.queue.length) await this.queue.shift()!.run();
    this.busy = false;
  }

  /** Starts evaluating every sample of these shapes (the nearest to `frame` first). */
  prefetch(shapes: FlexShape[], frame = 0) {
    for (const f of shapes) {
      const s = this.samplesOf(f).sort((a, b) => Math.abs(a - frame) - Math.abs(b - frame));
      s.forEach((k, i) => void this.load(f, k, i === 0));
    }
  }

  /** The meshes at fractional frame `frame`: the two nearest evaluated samples, blended. */
  at(shapes: FlexShape[], frame: number): FlexPose {
    const out: FlexPose = new Map();
    for (const f of shapes) {
      const s = this.samplesOf(f);
      const x = Math.max(0, Math.min(f.values.length - 1, frame));
      let hi = s.findIndex((k) => k >= x);
      if (hi < 0) hi = s.length - 1;
      const lo = Math.max(0, x === s[hi] ? hi : hi - 1);
      const [ka, kb] = [s[lo], s[hi]];
      // Wanted now: these two first.
      void this.load(f, ka, true);
      void this.load(f, kb, true);
      const get = (k: number) => this.meshes.get(this.sampleKey(f, f.values[k]));
      const [ma, mb] = [get(ka), get(kb)];
      const t = kb > ka ? (x - ka) / (kb - ka) : 0;
      if (ma && mb) out.set(flexKey(f.path, f.shape), { a: ma.mesh, b: mb.mesh, t, parts: ma.parts });
      else if (ma || mb) out.set(flexKey(f.path, f.shape), { a: (ma ?? mb)!.mesh, b: null, t: 0, parts: (ma ?? mb)!.parts });
      else {
        // Nothing near loaded yet: the nearest sample that is.
        const near = s.map((k) => [k, get(k)] as const).filter(([, m]) => m).sort((p, q) => Math.abs(p[0] - x) - Math.abs(q[0] - x))[0];
        if (near) out.set(flexKey(f.path, f.shape), { a: near[1]!.mesh, b: null, t: 0, parts: near[1]!.parts });
      }
    }
    return out;
  }

  /** Resolves once the meshes at `frame` are evaluated (for exporting frames). */
  ready(shapes: FlexShape[], frame: number): Promise<void> {
    const waits: Promise<void>[] = [];
    for (const f of shapes) {
      const s = this.samplesOf(f);
      const x = Math.max(0, Math.min(f.values.length - 1, frame));
      const hi = Math.max(0, s.findIndex((k) => k >= x));
      for (const k of new Set([s[Math.max(0, hi - 1)], s[hi]])) waits.push(this.load(f, k, true));
    }
    return Promise.all(waits).then(() => undefined);
  }
}
