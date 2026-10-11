// A CoreXY FDM printer. The frame is cut 2020 T-slot extrusion (true slot
// profile), the motion runs on MGN linear rails, the bed rides a lead screw
// on two rods at the back. `printHeight` moves the bed: 0 is the first layer.
// Origin at the front-left-bottom corner of the frame; Z up.
const W = p.build + 200, D = p.build + 200, H = p.build + 260;
const e = 20; // extrusion size
const parts = [];
const alu = { material: "Aluminium" };
const printed = { material: "PETG", process: "3d_print" };
const bought = { process: "buy" };

// ---- 2020 profile: 20 × 20, four 6.2 mm slots opening to 11 mm T-chambers, Ø4.2 core.
function profile2020() {
  let s = rect(e, e, { center: true });
  const slot = rect(6.2, 2.2, { center: true }).translate(0, e / 2 - 1.1 + 0.01)
    .add(polygon([[-5.5, e / 2 - 2.2], [5.5, e / 2 - 2.2], [5.5, e / 2 - 4.3], [3, e / 2 - 6.1], [-3, e / 2 - 6.1], [-5.5, e / 2 - 4.3]]));
  for (const a of [0, 90, 180, 270]) s = s.subtract(slot.rotate(a));
  return s.subtract(circle(2.1));
}
const P = profile2020();
/** An extrusion of length L along an axis, its 20 × 20 section's min corner at (x, y, z). */
function extrusion(axis, L, x, y, z) {
  const bar = P.extrude(L).translate(e / 2, e / 2, 0); // along Z, section in [0, 20]²
  const placed = axis === "z" ? bar : axis === "x" ? bar.rotate(0, 90, 0).translate(0, 0, e) : bar.rotate(-90, 0, 0).translate(0, 0, e);
  return placed.translate(x, y, z);
}
const ext = { ...alu, process: "extrusion" };
// Bottom and top rings: X rails run the full width, Y rails fit between them.
for (const [lvl, z] of [["Bottom", 0], ["Top", H - e]]) {
  parts.push(part(`${lvl} front rail`, extrusion("x", W, 0, 0, z), ext));
  parts.push(part(`${lvl} back rail`, extrusion("x", W, 0, D - e, z), ext));
  parts.push(part(`${lvl} left rail`, extrusion("y", D - 2 * e, 0, e, z), ext));
  parts.push(part(`${lvl} right rail`, extrusion("y", D - 2 * e, W - e, e, z), ext));
}
for (const [name, x, y] of [["front left", 0, 0], ["front right", W - e, 0], ["back left", 0, D - e], ["back right", W - e, D - e]]) {
  parts.push(part(`Post ${name}`, extrusion("z", H - 2 * e, x, y, e), ext));
}
// Corner gussets: printed triangles where each post meets its X rails.
const gusset = polygon([[0, 0], [20, 0], [0, 20]]).extrude(e); // in XY, then stood up in XZ
let gi = 1;
for (const [x, y] of [[e, 0], [W - e, 0], [e, D - e], [W - e, D - e]]) {
  const flip = x > W / 2;
  const tri = gusset.rotate(90, 0, 0).translate(0, e, 0); // XZ triangle, Y in [0, 20]
  const bottom = (flip ? tri.mirror(1, 0, 0) : tri).translate(x, y, e);
  const top = (flip ? tri.mirror(1, 0, 0) : tri).mirror(0, 0, 1).translate(x, y, H - e);
  parts.push(part(`Gusset ${gi++}`, bottom, printed), part(`Gusset ${gi++}`, top, printed));
}

// ---- Base: aluminium plate under the frame, rubber feet, PSU and electronics.
parts.push(part("Base plate", box(W, D, 3).translate(0, 0, -3), { ...alu, process: "laser" }));
for (const [i, [x, y]] of [[15, 15], [W - 15, 15], [15, D - 15], [W - 15, D - 15]].entries()) {
  parts.push(part(`Foot ${i + 1}`, cylinder(10, 12).translate(x, y, -13), { material: "Rubber", ...bought }));
}
// Bought-in parts are their envelopes: the PSU is its 1 mm steel case; motors use
// an effective density that gives the catalogue mass (NEMA 17, 0.35 kg).
const psu = box(215, 115, 50).subtract(box(213, 113, 48).translate(1, 1, 1));
parts.push(part("Power supply", psu.translate(e + 10, e + 10, 0), { material: "Steel", ...bought }));
const ebox = box(130, 100, 40).subtract(box(126, 96, 40).translate(2, 2, 2));
parts.push(part("Electronics enclosure", ebox.translate(W - e - 10 - 130, e + 10, 0), printed));
parts.push(part("Mainboard", box(100, 70, 1.6).translate(W - e - 10 - 130 + 15, e + 10 + 15, 2), { material: "FR4", ...bought }));

// ---- Z axis: lead screw between two rods at the back, motor below.
const zx = W / 2, zy = D - 45;
const rodX = [zx - 60, zx + 60];
const mountL = box(170, 42, 30).translate(zx - 85, D - e - 42, 0);
const mountHoles = rodX.map((x) => cylinder(32, 4.05).translate(x, zy, -1));
parts.push(part("Z lower mount", mountL.subtract(...mountHoles).subtract(box(44, 46, 32).translate(zx - 22, D - e - 44, -1)), printed));
parts.push(part("Z upper mount", box(170, 42, 30).translate(zx - 85, D - e - 42, H - e - 30).subtract(...rodX.map((x) => cylinder(32, 4.05).translate(x, zy, H - e - 31)), cylinder(32, 6).translate(zx, zy, H - e - 31)), printed));
for (const [i, x] of rodX.entries()) parts.push(part(`Z rod ${i ? "right" : "left"}`, cylinder(H - e - 30 - 1, 4).translate(x, zy, 1), { material: "Steel", ...bought }));
parts.push(part("Z motor", box(42, 42, 40).translate(zx - 21, zy - 21, 0), { material: "Stepper motor", ...bought }));
parts.push(part("Z coupler", cylinder(25, 10).translate(zx, zy, 40), { material: "Aluminium", ...bought }));
parts.push(part("Lead screw", cylinder(H - e - 30 - 65, 4).translate(zx, zy, 65), { material: "Steel", ...bought }));

// ---- Toolhead position and the bed below it.
const yCar = D / 2 + 31; // X beam centre line; the nozzle hangs 31 mm in front of it
const railTop = H + 8;
const nozzleTip = H - 55;
const bedTop = nozzleTip - 0.3 - p.printHeight;
const bed = p.build + 15;
parts.push(part("Bed plate", box(bed, bed, 6).translate(W / 2 - bed / 2, D / 2 - bed / 2, bedTop - 6), { ...alu, process: "laser" }));
// Bed yoke: plate under the bed reaching back to the rods and screw.
const yokeZ = bedTop - 6 - 8 - 8;
const yoke = rect(200, 200, { center: true }).translate(W / 2, D / 2)
  .add(rect(160, zy + 14 - (D / 2 + 100)).translate(zx - 80, D / 2 + 100))
  .offset(-6).offset(6)
  .subtract(...rodX.map((x) => circle(4.6).translate(x, zy)), circle(5.2).translate(zx, zy))
  .extrude(8).translate(0, 0, yokeZ);
parts.push(part("Bed yoke", yoke, { ...alu, process: "laser" }));
for (const [i, [x, y]] of [[-85, -85], [85, -85], [-85, 85], [85, 85]].entries()) {
  parts.push(part(`Bed spacer ${i + 1}`, cylinder(8, 6).translate(W / 2 + x, D / 2 + y, yokeZ + 8), { material: "Silicone", ...bought }));
}
const tubeRing = (od, id, h) => cylinder(h, od / 2).subtract(cylinder(h + 2, id / 2).translate(0, 0, -1));
for (const [i, x] of rodX.entries()) parts.push(part(`Linear bearing ${i ? "right" : "left"}`, tubeRing(15, 8.1, 24).translate(x, zy, yokeZ - 24), { material: "Steel", ...bought }));
parts.push(part("Lead screw nut", tubeRing(22, 8.1, 3.5).add(tubeRing(10.2, 8.1, 15).translate(0, 0, -15)).translate(zx, zy, yokeZ - 3.5), { material: "Brass", ...bought }));

// ---- CoreXY motion: Y rails on the top side rails, X beam on their carriages.
const rail = (L) => box(12, L, 8);
for (const [side, x] of [["left", e / 2], ["right", W - e / 2]]) {
  parts.push(part(`Y rail ${side}`, rail(D - 60).translate(x - 6, 30, H), { material: "Steel", ...bought }));
  const block = box(27, 45, 10).subtract(box(12.5, 47, 5.5).translate(7.25, -1, -0.01));
  parts.push(part(`Y carriage ${side}`, block.translate(x - 13.5, yCar - 22.5, H + 3), { material: "Steel", ...bought }));
}
parts.push(part("X beam", extrusion("x", W, 0, yCar - e / 2, H + 13), ext));
parts.push(part("X rail", box(W - 80, 9, 6.5).translate(40, yCar - 4.5, H + 33), { material: "Steel", ...bought }));
const xBlock = box(30, 20, 8).subtract(box(32, 9.5, 4.5).translate(-1, 5.25, -0.01));
parts.push(part("X carriage", xBlock.translate(W / 2 - 15, yCar - 10, H + 36), { material: "Steel", ...bought }));

// Toolhead: a printed shroud bolted to the X carriage, hanging in front of the beam.
const tz = H + 44;
const shroud = box(50, 51 + 10, 6).translate(W / 2 - 25, yCar - 51, tz)
  .add(box(50, 40, tz - (H - 35)).translate(W / 2 - 25, yCar - 51, H - 35))
  .subtract(cylinder(80, 11).translate(W / 2, yCar - 31, H - 36));
parts.push(part("Toolhead shroud", shroud, printed));
parts.push(part("Heatsink", cylinder(40, 10.9).subtract(cylinder(42, 2).translate(0, 0, -1)).translate(W / 2, yCar - 31, H - 35 + 2), { material: "Aluminium", ...bought }));
parts.push(part("Heater block", box(20, 16, 11.5).translate(W / 2 - 10, yCar - 31 - 8, H - 46.5), { ...alu, process: "cnc_mill" }));
// Nozzle: cone to a 0.4 mm tip, 7 mm hex, then thread up into the heater block's underside.
const nozzle = cylinder(2, 0.4, { top: 3.2 })
  .add(cylinder(3, 4, { segments: 6 }).translate(0, 0, 2))
  .add(cylinder(H - 46.5 - (nozzleTip + 5), 3).translate(0, 0, 5))
  .translate(W / 2, yCar - 31, nozzleTip);
parts.push(part("Nozzle", nozzle, { material: "Brass", process: "lathe" }));

// ---- XY motors at the back corners, on the back face of the top back rail.
for (const [side, x] of [["A", 10], ["B", W - 52]]) {
  parts.push(part(`XY motor ${side}`, box(42, 42, 40).translate(x, D, H - 40), { material: "Stepper motor", ...bought }));
}

return parts;
