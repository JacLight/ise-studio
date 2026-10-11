// A work desk as it is made: a CNC-cut birch ply top, two welded steel leg
// frames, a bolted stretcher, a slotted modesty panel, a folded cable tray and
// bought-in glides. Origin at the floor, centred, front edge toward −Y.
const W = p.width, D = p.depth, H = p.height, T = p.top;
const tube = p.tube; // square tube section of the leg frames
const wall = p.tubeWall;
const glide = 12; // glide height under the feet
const legInset = p.legInset; // leg frame centre line from the desk ends
const frameTop = H - T; // the top rests on the frames' top arms
const parts = [];

/** A hollow rectangular tube along an axis, capped at both ends (welded caps). */
function rectTube(len, a, b, axis) {
  const outer = box(len, a, b);
  const inner = box(len - 2 * wall, a - 2 * wall, b - 2 * wall).translate(wall, wall, wall);
  const t = outer.subtract(inner);
  if (axis === "x") return t;
  if (axis === "y") return t.rotate(0, 0, 90).translate(a, 0, 0);
  return t.rotate(0, -90, 0).translate(b, 0, 0); // along z
}

// Leg frame: foot and top arm along the depth, an upright between them. One welded part.
function legFrame(x) {
  const depthSpan = D - 2 * p.armInset;
  const y0 = -D / 2 + p.armInset;
  const foot = rectTube(depthSpan, tube, tube, "y").translate(x - tube / 2, y0, glide);
  const arm = rectTube(depthSpan, tube, tube, "y").translate(x - tube / 2, y0, frameTop - tube);
  const upright = rectTube(frameTop - 2 * tube - glide, tube, tube, "z").translate(x - tube / 2, -tube / 2, glide + tube);
  // Threaded mounting bosses are drilled through the arm's top face (4 per arm).
  return union([foot, arm, upright]);
}
const xl = -W / 2 + legInset, xr = W / 2 - legInset;
const steel = { material: "Powder-coated steel" };
parts.push(part("Left leg frame", legFrame(xl), { ...steel, process: "weld" }));
parts.push(part("Right leg frame", legFrame(xr), { ...steel, process: "weld" }));

// Stretcher between the uprights, bolted through end plates.
const sh = 40, sd = 20;
parts.push(
  part("Stretcher", rectTube(xr - xl - tube, sd, sh, "x").translate(xl + tube / 2, -sd / 2, glide + (frameTop - glide) * 0.42), { ...steel, process: "extrusion" }),
);

// Glides under each end of the feet.
const yEnds = [-D / 2 + p.armInset + 25, D / 2 - p.armInset - 25];
let g = 1;
for (const x of [xl, xr]) {
  for (const y of yEnds) parts.push(part(`Glide ${g++}`, cylinder(glide, 14).translate(x, y, 0), { material: "Rubber", process: "buy" }));
}

// Top: rounded corners, a 60 mm grommet at the back, and Ø8 × 14 mm pockets
// for threaded inserts over each arm (machined from the underside).
let top = rect(W, D, { center: true }).offset(-p.corner).offset(p.corner).extrude(T).translate(0, 0, frameTop);
const grommet = cylinder(T + 2, 30).translate(0, D / 2 - 90, frameTop - 1);
const inserts = [];
for (const x of [xl, xr]) {
  for (const f of [0.15, 0.38, 0.62, 0.85]) {
    const y = -D / 2 + p.armInset + f * (D - 2 * p.armInset);
    inserts.push(cylinder(15, 4).translate(x, y, frameTop - 1));
  }
}
top = top.subtract(grommet, ...inserts);
parts.push(part("Desktop", top, { material: "Birch ply", process: "cnc_router" }));
parts.push(
  part("Grommet", cylinder(T, 30).subtract(cylinder(T + 2, 26).translate(0, 0, -1)).add(cylinder(3, 34).subtract(cylinder(5, 26).translate(0, 0, -1)).translate(0, 0, T)).translate(0, D / 2 - 90, frameTop - 0.0), { material: "PLA", process: "3d_print" }),
);

// Modesty panel bolted across the back faces of the uprights, with ventilation slots.
const mpH = p.modesty, mpT = 18;
const mpY = tube / 2; // against the uprights' back faces
let panel = box(xr - xl + tube, mpT, mpH).translate(xl - tube / 2, mpY, frameTop - tube - mpH - 30);
const slots = [];
const n = Math.floor((xr - xl - tube - 200) / 60);
for (let i = 0; i < n; i++) {
  const x = xl + tube / 2 + 100 + i * 60 + 20;
  slots.push(rect(20, mpH * 0.5).offset(-9.9).offset(9.9).extrude(mpT + 2).rotate(90, 0, 0).translate(x, mpY + mpT + 1, frameTop - tube - mpH - 30 + mpH * 0.25));
}
panel = panel.subtract(union(slots));
parts.push(part("Modesty panel", panel, { material: "Birch ply", process: "cnc_router" }));

// Cable tray: a folded steel channel hung under the top's back edge.
const trayL = xr - xl - tube - 40, trayW = 120, trayH = 80, gauge = 1.5;
const tray = box(trayL, trayW, trayH)
  .subtract(box(trayL + 2, trayW - 2 * gauge, trayH).translate(-1, gauge, gauge))
  .translate(-trayL / 2, D / 2 - 260, frameTop - trayH);
parts.push(part("Cable tray", tray, { ...steel, process: "sheet_metal" }));

return parts;
