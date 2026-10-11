// A run of frameless (European, 32 mm system) kitchen cabinets: every panel
// is cut on a CNC router with its joinery machined in: back-panel dados,
// shelf-pin holes on the 32 mm grid, 35 mm hinge cups in the doors. Base
// cabinets stand on adjustable legs behind a toe kick; a quartz top with an
// undermount sink cut-out spans the run; wall cabinets hang above.
// X along the wall, Y from the front (0) to the wall (+), Z up from the floor.
const t = p.panel; // carcass panels
const bt = 6; // back panel
const dadoW = bt + 0.5, dadoD = 8, backset = 15; // back-panel groove, set in from the rear
const pinR = 2.5, pinDepth = 12, pinRow = 37; // shelf pins: Ø5 × 12, 37 mm from the edges
const gap = 1.5; // half the reveal between doors
const parts = [];
const ply = { material: "Birch ply", process: "cnc_router" };
const doorMat = { material: "Painted MDF", process: "cnc_router" };

/**
 * One carcass at x0 (left), y0 (front), z0 (bottom). Returns its parts.
 * name: e.g. "B2"; shelves: count; doors: 1 or 2.
 */
function carcass(name, x0, y0, z0, w, d, h, { shelves = 1, doors = 1 }) {
  const out = [];
  const backY = d - backset - dadoW; // front face of the back groove
  const add = (label, solid, kind = ply) => out.push(part(`${name} ${label}`, solid.translate(x0, y0, z0), kind));

  // Sides: back groove on the inner face, two columns of shelf-pin holes.
  const pinZs = [];
  for (let z = 160; z <= h - 160; z += 32) pinZs.push(z);
  // A Ø5 bore along X, `len` long, starting at x.
  const bore = (x, y, z, len, r = pinR) => cylinder(len, r).rotate(0, 90, 0).translate(x, y, z);
  const side = (left) => {
    const x = left ? 0 : w - t;
    const face = left ? t : w - t; // the inner face
    let s = box(t, d, h).translate(x, 0, 0);
    s = s.subtract(box(dadoD, dadoW, h + 2).translate(left ? face - dadoD : face, backY, -1));
    const holes = [];
    for (const z of pinZs) {
      for (const y of [pinRow, backY - pinRow]) holes.push(left ? bore(face - pinDepth, y, z, pinDepth + 1) : bore(face - 1, y, z, pinDepth + 1));
    }
    return s.subtract(union(holes));
  };
  add("left side", side(true));
  add("right side", side(false));

  // Bottom between the sides, with the same back groove on its top face.
  const inner = w - 2 * t;
  add("bottom", box(inner, d, t).translate(t, 0, 0).subtract(box(inner + 2, dadoW, dadoD).translate(t - 1, backY, t - dadoD)));
  // Rails across the top: one at the front, one in front of the back panel.
  add("front rail", box(inner, 100, t).translate(t, 0, h - t));
  add("back rail", box(inner, 100, t).translate(t, backY - 100, h - t));
  // Back panel, sliding in the grooves with 0.25 mm play each side.
  add("back", box(inner + 2 * dadoD - 0.5, bt, h - (t - dadoD) - 0.25).translate(t - dadoD + 0.25, backY + 0.25, t - dadoD + 0.25));

  // Shelves rest on pins (bought in) in the holes nearest the middle.
  const usable = pinZs.slice(1, -1);
  for (let i = 0; i < shelves; i++) {
    const zPin = usable[Math.round(((i + 1) * (usable.length - 1)) / (shelves + 1))];
    const pinTop = zPin + 2.45;
    add(`shelf ${i + 1}`, box(inner - 2, backY - 2 - 20, t).translate(t + 1, 20, pinTop));
    // Pins sit 12 mm in their holes and stand 4 mm proud, under the shelf.
    for (const left of [true, false]) {
      for (const y of [pinRow, backY - pinRow]) {
        const pin = bore(left ? t - pinDepth : w - t - 4, y, zPin, pinDepth + 4, 2.45);
        out.push(part(`${name} shelf ${i + 1} pin ${left ? "L" : "R"}${y === pinRow ? "F" : "B"}`, pin.translate(x0, y0, z0), { material: "Steel", process: "buy" }));
      }
    }
  }

  // Doors in front of the carcass: 18 mm painted MDF with Ø35 × 12.5 hinge cups.
  const doorT = 18;
  const dw = (w - 2 * gap * doors) / doors;
  for (let i = 0; i < doors; i++) {
    const dx = gap + i * (dw + 2 * gap);
    let door = box(dw, doorT, h - 2 * gap).translate(dx, -doorT - 2, gap);
    const hingeX = doors === 2 && i === 1 ? dx + dw - 22.5 : dx + 22.5;
    const cups = [100, h - 2 * gap - 100].map((z) => cylinder(13.5, 17.5).rotate(-90, 0, 0).translate(hingeX, -2 - 12.5, gap + z));
    door = door.subtract(...cups);
    add(doors === 1 ? "door" : i === 0 ? "left door" : "right door", door, doorMat);
  }
  return out;
}

// Base run.
const widths = [p.base1, p.base2, p.base3];
const D = p.depth, H = p.baseHeight, leg = p.legHeight;
let x = 0;
widths.forEach((w, i) => {
  const name = `B${i + 1}`;
  const sink = i === 1;
  parts.push(...carcass(name, x, 0, leg, w, D, H, { shelves: sink ? 0 : 1, doors: w >= 800 ? 2 : 1 }));
  // Four adjustable legs, 50 mm in from the corners.
  let n = 1;
  for (const lx of [50, w - 50]) {
    for (const ly of [70, D - 70]) {
      parts.push(part(`${name} leg ${n++}`, cylinder(leg, 18).translate(x + lx, ly, 0), { material: "Nylon", process: "buy" }));
    }
  }
  x += w;
});
const run = x;

// Toe kick across the run, clipped to the front legs.
parts.push(part("Toe kick", box(run, 16, leg - 3).translate(0, 70 - 18 - 16, 3), { material: "Painted MDF", process: "cnc_router" }));

// Quartz worktop: overhangs the doors, undermount sink cut-out over B2, tap hole.
const topT = p.worktop;
const front = -20 - 22; // 22 mm past the door faces
let worktop = box(run, D - front, topT).translate(0, front, leg + H);
const sinkX = widths[0] + widths[1] / 2;
const sinkCut = rect(p.sinkWidth, p.sinkDepth, { center: true }).offset(-25).offset(25).extrude(topT + 2).translate(sinkX, front + (D - front) / 2 - 20, leg + H - 1);
const tap = cylinder(topT + 2, 17.5).translate(sinkX, front + (D - front) / 2 + p.sinkDepth / 2 + 35, leg + H - 1);
worktop = worktop.subtract(sinkCut, tap);
parts.push(part("Worktop", worktop, { material: "Quartz", process: "laser" }));

// Wall cabinets, backs flush with the base cabinets', hung from a rail.
const wallD = p.wallDepth, wallH = p.wallHeight, wallZ = p.wallBottom;
x = 0;
widths.forEach((w, i) => {
  parts.push(...carcass(`W${i + 1}`, x, D - wallD, wallZ, w, wallD, wallH, { shelves: 2, doors: w >= 800 ? 2 : 1 }));
  x += w;
});

return parts;
