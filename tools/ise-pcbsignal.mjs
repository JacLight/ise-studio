// `./ise pcb netclass|pair|group|lengths|tune|route-pair|pad-to-die …`: routing
// constraints — impedance-targeted net classes (widths per layer from the
// stackup), differential pairs, length-matching groups, net lengths, length
// tuning and pair routing — through the same engine commands the Electronics
// workspace uses. Called from tools/ise.mjs with its engine connection.

export const SIGNAL_SUBS = ["netclass", "pair", "group", "lengths", "tune", "route-pair", "pad-to-die"];

const flagOf = (args, name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args.splice(i, 2)[1] : null;
};
const boolOf = (args, name) => {
  const i = args.indexOf(name);
  if (i >= 0) args.splice(i, 1);
  return i >= 0;
};
const num = (v, what) => {
  const x = Number(v);
  if (!Number.isFinite(x)) throw new Error(`${what}: “${v}” is not a number`);
  return x;
};
const mm = (x, d = 3) => (x == null ? "—" : `${(+x).toFixed(d)}`);

export async function pcbSignal(sub, args, { engine, get, http, q }) {
  const commit = async (op, params) => {
    const r = await engine.commit(op, params);
    if (!r.ok) throw new Error(`${op}: ${r.error.code} ${r.error.message}`);
    return r.value.output;
  };
  const view = async (board) => (await get(`/pcb/${board}?${q()}`)).board;
  const preview = async (board, body) => {
    const res = await fetch(`${http}/pcb/${board}/router?${q()}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    if (!res.ok) throw new Error(await res.text());
    return res.json();
  };
  const showClass = (c) => {
    console.log(`${c.name}: ${c.impedance ? `${c.impedance} Ω single-ended` : ""}${c.impedance && c.diff_impedance ? ", " : ""}${c.diff_impedance ? `${c.diff_impedance} Ω differential` : ""} ±${c.tolerance} %`);
    for (const l of c.layers) {
      const m = l.line.model === "microstrip" ? `microstrip h ${l.line.h} εr ${l.line.er}` : `stripline h ${l.line.h1}/${l.line.h2} εr ${l.line.er1}/${l.line.er2}`;
      console.log(`  ${l.layer.padEnd(7)} width ${mm(l.width)} mm  Z0 ${l.z0.toFixed(1)} Ω${l.gap != null ? `  gap ${mm(l.gap)} mm  Zdiff ${l.zdiff.toFixed(1)} Ω` : ""}  ${l.delay.toFixed(2)} ps/mm  (${m})`);
      for (const n of l.notes ?? []) console.log(`          ! ${n}`);
    }
  };
  const showPair = (p) =>
    console.log(`  ${p.name.padEnd(10)} ${p.p} / ${p.n}${p.detected ? " (from names)" : ""}  [${p.class}]  ${mm(p.length_p)} / ${mm(p.length_n)} mm  skew ${mm(p.skew)}${p.max_skew != null ? ` (≤ ${p.max_skew})` : ""}  coupled ${mm(p.coupled, 2)}  uncoupled ${mm(p.uncoupled, 2)}${p.max_uncoupled != null ? ` (≤ ${p.max_uncoupled})` : ""}${p.gap_min != null ? `  gap ${mm(p.gap_min)}–${mm(p.gap_max)}` : ""}${p.gap != null ? ` (wants ${mm(p.gap)})` : ""}${p.zdiff != null ? `  ${p.zdiff} Ω` : ""}${p.routed ? "" : "  (not fully routed)"}`);
  const violations = (v) => v.violations.filter((x) => ["impedance", "diff_pair_gap", "diff_pair_uncoupled", "length", "skew", "via_count"].includes(x.code));

  const board = args[2];
  if (!board) throw new Error(`usage: ise pcb ${sub} <board> …`);

  if (sub === "netclass") {
    const name = args[3];
    if (!name) {
      const v = await view(board);
      for (const c of v.signal.classes) showClass(c);
      if (!v.signal.classes.length) console.log("no net class has an impedance target (ise pcb netclass <board> <name> --impedance 50 | --diff 90)");
      return false;
    }
    const params = { board, name };
    const map = { "--width": "track_width", "--clearance": "clearance", "--via-drill": "via_drill", "--via-diameter": "via_diameter", "--neck": "neck_width", "--impedance": "impedance", "--diff": "diff_impedance", "--gap": "diff_gap", "--tol": "impedance_tolerance", "--max-uncoupled": "max_uncoupled", "--max-skew": "max_skew", "--via-pitch": "pair_via_pitch" };
    for (const [f, k] of Object.entries(map)) {
      const x = flagOf(args, f);
      if (x != null) params[k] = num(x, f);
    }
    const nets = flagOf(args, "--nets");
    const add = flagOf(args, "--add");
    const patterns = flagOf(args, "--patterns");
    const rename = flagOf(args, "--rename");
    if (nets != null) params.nets = nets.split(",").filter(Boolean);
    if (add != null) params.add_nets = add.split(",").filter(Boolean);
    if (patterns != null) params.patterns = patterns.split(",").filter(Boolean);
    if (rename) params.rename = rename;
    if (boolOf(args, "--delete")) params.delete = true;
    const r = await commit("pcb.netclass.set", params);
    if (r.deleted) {
      console.log(`✓ deleted ${r.deleted}`);
      return false;
    }
    console.log(`✓ ${r.class.name}: ${r.class.nets.length} net(s)${r.class.patterns?.length ? `, patterns ${r.class.patterns.join(" ")}` : ""}; ${r.class.track_width} mm default, clearance ${r.class.clearance} mm`);
    if (r.layers.length) showClass({ ...r.class, tolerance: r.class.impedance_tolerance ?? 10, layers: r.layers });
    return false;
  }
  if (sub === "pair") {
    const name = args[3];
    if (!name) {
      const v = await view(board);
      console.log(`${v.signal.pairs.length} differential pair(s)${v.signal.detect_pairs ? " (found from names too)" : ""}:`);
      for (const p of v.signal.pairs) showPair(p);
      return false;
    }
    const params = { board, name };
    if (boolOf(args, "--delete")) params.delete = true;
    const ms = flagOf(args, "--max-skew");
    const mu = flagOf(args, "--max-uncoupled");
    if (ms != null) params.max_skew = num(ms, "--max-skew");
    if (mu != null) params.max_uncoupled = num(mu, "--max-uncoupled");
    if (args[4]) params.p = args[4];
    if (args[5]) params.n = args[5];
    const r = await commit("pcb.pair.set", params);
    console.log(r.deleted ? `✓ deleted ${r.deleted}` : `✓ pair ${r.pair.name}: ${r.pair.p} / ${r.pair.n}`);
    return false;
  }
  if (sub === "group") {
    const name = args[3];
    if (!name) throw new Error("usage: ise pcb group <board> <name> <net,net,… | D? | pair> [--target mm] [--tol mm] [--max-skew mm] [--min mm] [--max mm] | --delete");
    const params = { board, name };
    if (boolOf(args, "--delete")) params.delete = true;
    for (const [f, k] of Object.entries({ "--target": "target", "--tol": "tolerance", "--max-skew": "max_skew", "--min": "min", "--max": "max" })) {
      const x = flagOf(args, f);
      if (x != null) params[k] = num(x, f);
    }
    if (args[4]) params.nets = args[4].split(",").filter(Boolean);
    const r = await commit("pcb.length.group", params);
    console.log(r.deleted ? `✓ deleted ${r.deleted}` : `✓ group ${r.group.name}: ${r.group.nets.join(", ")}`);
    return false;
  }
  if (sub === "lengths") {
    const v = await view(board);
    const s = v.signal;
    const filter = args[3];
    console.log("Net lengths (track + vias + pad-to-die):");
    for (const l of s.lengths.filter((l) => !filter || l.net.includes(filter))) console.log(`  ${l.net.padEnd(12)} ${mm(l.length).padStart(9)} mm  track ${mm(l.tracks)}  ${l.vias} via(s) ${mm(l.via_length)}${l.pad_to_die ? `  die ${mm(l.pad_to_die)}` : ""}  ${l.delay.toFixed(1)} ps${l.routed ? "" : "  (unrouted)"}`);
    if (s.pairs.length) console.log("Pairs:");
    for (const p of s.pairs) showPair(p);
    if (s.groups.length) console.log("Length groups:");
    for (const g of s.groups) console.log(`  ${g.name}: target ${mm(g.target)} mm${g.tolerance != null ? ` ± ${g.tolerance}` : ""}${g.max_skew != null ? `, skew ≤ ${g.max_skew}` : ""}; spread ${mm(g.skew)} mm — ${g.members.map((m) => `${m.net} ${mm(m.length)}`).join(", ")}`);
    const bad = violations(v);
    for (const x of bad) console.log(`  ${x.severity === "error" ? "✗" : "!"} ${x.code}: ${x.message}`);
    console.log(`${bad.filter((x) => x.severity === "error").length} routing-constraint error(s)`);
    return bad.some((x) => x.severity === "error");
  }
  if (sub === "pad-to-die") {
    const pad_to_die = {};
    for (const a of args.slice(3)) {
      const [k, x] = a.split("=");
      pad_to_die[k] = num(x, k);
    }
    const r = await commit("pcb.signal.set", { board, pad_to_die });
    console.log(`✓ pad-to-die: ${Object.entries(r.signal.pad_to_die ?? {}).map(([k, x]) => `${k} ${x} mm`).join(", ") || "none"}`);
    return false;
  }
  if (sub === "tune") {
    const what = args[3];
    if (!what) throw new Error("usage: ise pcb tune <board> <net | pair | group> [--target mm] [--skew] [--amplitude mm] [--spacing mm] [--radius mm] [--style trombone|sawtooth] [--corner round|chamfer|square] [--side left|right] [--tol mm] [--track id] [--segment n] [--dry]");
    const v = await view(board);
    const params = { board };
    if (v.signal.groups.some((g) => g.name.toLowerCase() === what.toLowerCase())) params.group = what;
    else if (v.signal.pairs.some((p) => p.name.toLowerCase() === what.toLowerCase())) params.pair = what;
    else params.net = what;
    for (const [f, k] of Object.entries({ "--target": "target", "--amplitude": "amplitude", "--spacing": "spacing", "--radius": "radius", "--tol": "tolerance", "--segment": "segment" })) {
      const x = flagOf(args, f);
      if (x != null) params[k] = num(x, f);
    }
    for (const [f, k] of Object.entries({ "--style": "style", "--corner": "corner", "--side": "side", "--track": "track" })) {
      const x = flagOf(args, f);
      if (x != null) params[k] = x;
    }
    if (boolOf(args, "--skew")) params.mode = "skew";
    const dry = boolOf(args, "--dry");
    const r = dry ? await preview(board, { op: "tune", ...params }) : await commit("pcb.tune", params);
    console.log(`${r.ok ? "✓" : "✗"} ${dry ? "would tune" : "tuned"} ${r.edits.length} track(s)${r.skew != null ? `; skew ${mm(r.skew, 4)} mm` : ""}`);
    for (const n of r.nets) console.log(`  ${n.net.padEnd(12)} ${mm(n.before)} → ${mm(n.after, 4)} mm (target ${mm(n.target, 4)})`);
    for (const n of r.notes) console.log(`  · ${n}`);
    return !r.ok;
  }
  if (sub === "route-pair") {
    const pair = args[3];
    if (!pair) throw new Error("usage: ise pcb route-pair <board> <pair> [--from J1.A6,J1.A7] [--to U1.3,U1.4] [--step x,y[:layer]]… [--mode walkaround|shove] [--width mm] [--gap mm] [--via-pitch mm] [--arc] [--dry]");
    const params = { board, pair, steps: [] };
    const from = flagOf(args, "--from");
    const to = flagOf(args, "--to");
    if (from) params.from = from.split(",");
    if (to) params.to = to.split(",");
    let s;
    while ((s = flagOf(args, "--step"))) {
      const [xy, layer] = s.split(":");
      const at = xy.split(",").map(Number);
      params.steps.push({ at, ...(layer ? { layer } : {}) });
    }
    for (const [f, k] of Object.entries({ "--width": "width", "--gap": "gap", "--via-pitch": "via_pitch" })) {
      const x = flagOf(args, f);
      if (x != null) params[k] = num(x, f);
    }
    const mode = flagOf(args, "--mode");
    if (mode) params.mode = mode;
    if (boolOf(args, "--arc")) params.corners = "arc";
    const dry = boolOf(args, "--dry");
    if (dry) {
      const r = await preview(board, { op: "pair", ...params });
      console.log(`${r.ok ? "✓" : "✗"} would route ${r.pair}: ${r.width} mm / ${r.gap} mm gap, ${mm(r.length_p)} / ${mm(r.length_n)} mm, skew ${mm(r.skew, 4)}`);
      for (const c of r.p.collisions) console.log(`  ✗ ${c.layer ?? ""} too close to ${c.with}: ${c.gap} mm (needs ${c.need})`);
      for (const n of r.notes) console.log(`  · ${n}`);
      return !r.ok;
    }
    const r = await commit("pcb.route.pair", params);
    console.log(`${r.ok ? "✓" : "✗"} ${r.pair}: ${r.width} mm tracks, ${r.gap} mm gap; ${mm(r.length_p)} / ${mm(r.length_n)} mm, skew ${mm(r.skew, 4)} mm`);
    for (const c of r.collisions) console.log(`  ✗ ${c.layer ?? ""} too close to ${c.with}: ${c.gap} mm (needs ${c.need})`);
    for (const n of r.notes) console.log(`  · ${n}`);
    return !r.ok;
  }
  throw new Error(`unknown: ise pcb ${sub}`);
}
