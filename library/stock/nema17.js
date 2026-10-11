// NEMA 17 hybrid stepper motor, 1.8° (200 steps/rev), body 34 / 40 / 48 mm.
//
// Source: NEMA ICS 16 size 17 frame — 42.3 mm square flange, Ø22 mm × 2 mm
// pilot, M3 mounting holes on a 31.0 mm square pattern (about 4.5 mm deep),
// Ø5 mm shaft 24 mm from the mounting face with a 0.5 mm D-flat (4.5 mm
// across) over the outer 15 mm; generic 17HS-series datasheets (e.g.
// 17HS4401: 42×42×40 mm, 0.28 kg; 34 mm bodies ≈ 0.23 kg; 48 mm bodies
// ≈ 0.35 kg), 6-pin JST-PH socket on the rear cap's side.
//
// Shaft along +Z, the mounting face (front cap face) on z = 0, body behind
// it down −Z. Front and rear caps are aluminium; between them the stator
// stack with the rotor and windings inside is one lumped part whose density
// gives the datasheet mass (laminated silicon steel, copper, magnet).

const SQ = 42.3;
const CH = 4.6; // corner chamfer leg on the caps
const CAP = 8.5; // front and rear cap thickness
const L = p.length;
const HOLE = 31 / 2;

// The square with its corners chamfered: the end caps' outline. The stack
// between them is the same square with slightly larger chamfers, as the
// laminations are.
function outline(ch) {
  const h = SQ / 2;
  return polygon([
    [-h + ch, -h], [h - ch, -h], [h, -h + ch], [h, h - ch],
    [h - ch, h], [-h + ch, h], [-h, h - ch], [-h, -h + ch],
  ]);
}
const holes = [[HOLE, HOLE], [-HOLE, HOLE], [-HOLE, -HOLE], [HOLE, -HOLE]];

// Front cap with the four tapped M3 holes (ISO thread rings), and the pilot.
let front = outline(CH).extrude(CAP).translate(0, 0, -CAP);
for (const [x, y] of holes) {
  front = front.subtract(internalThreadTool({ d: 3, pitch: 0.5, z0: -4.5, z1: 0 }).translate(x, y, 0));
  front = front.subtract(cylinder(0.6, 1.25, { top: 0.001 }).translate(x, y, -4.5 - 0.6)); // drill point
}
const pilot = cylinder(2, 11);
front = front.add(pilot).subtract(cylinder(CAP + 3, 2.6).translate(0, 0, -CAP - 0.5));
// Rear cap, with the four through-bolt heads' counterbores at the corners.
let rear = outline(CH).extrude(CAP).translate(0, 0, -L);
for (const [x, y] of holes) rear = rear.subtract(cylinder(1.5, 2.8).translate(x, y, -L - 0.01));
// Stator stack between them.
const stack = outline(CH + 1.2).extrude(L - 2 * CAP).translate(0, 0, -L + CAP);
// Shaft: Ø5 from inside the front cap out to 24 mm, D-flat over the last 15.
const shaftLen = 24;
const flat = box(6, 6, 15.01).translate(-3, 2.0, shaftLen - 15);
const shaft = cylinder(shaftLen + 6, 2.5).translate(0, 0, -6).subtract(flat);
// JST-PH 6-pin socket on the −Y side of the rear cap.
const socket = box(16, 5.5, 6.5).translate(-8, -SQ / 2 - 5.5 + 0.01, -L + 1)
  .subtract(box(14.6, 4.5, 5.2).translate(-7.3, -SQ / 2 - 5.5 - 0.01, -L + 1.65));

const model = { 34: "17HS3401", 40: "17HS4401", 48: "17HS8401" }[L];
const typical = { 34: [230, 10.5], 40: [280, 11.5], 48: [350, 13.5] }[L];
if (!typical) throw new Error(`no NEMA 17 body length ${L} in the table (34, 40, 48)`);
const [massG, price] = typical;
stock({
  standard: "NEMA 17",
  partNumber: `NEMA17-42x${num(L)}-1.8`,
  description: `NEMA 17 stepper motor 42×42×${num(L)} mm, 1.8°, Ø5 mm D-shaft, 6-pin JST-PH (typ. ${model})`,
  mpn: L === 40 ? "17HS4401" : undefined,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("StepperOnline", `nema 17 42x${num(L)}`),
    supplierSearch("Amazon", `nema 17 stepper motor ${num(L)}mm ${model}`),
    supplierSearch("AliExpress", `nema17 stepper motor ${model} 42x${num(L)}`),
  ],
});
metric("Body length", L, "mm");
metric("Hole pattern", 31, "mm");
metric("Pilot diameter", 22, "mm");

connector("Shaft", { at: [0, 0, 0], axis: [0, 0, 1], x: [0, 1, 0], part: "Shaft" });
connector("Pilot", { at: [0, 0, 2], axis: [0, 0, 1], part: "Front cap" });
connector("Face", { at: [0, 0, 0], axis: [0, 0, 1], part: "Front cap" });
holes.forEach(([x, y], i) => connector(`Hole ${i + 1}`, { at: [x, y, 0], axis: [0, 0, 1], part: "Front cap" }));
connector("Rear face", { at: [0, 0, -L], axis: [0, 0, -1], part: "Rear cap" });

return [
  part("Front cap", front, { material: "Aluminium" }),
  part("Stator stack", stack, { material: "NEMA 17 stator, rotor and windings (lumped)" }),
  part("Rear cap", rear, { material: "Aluminium" }),
  part("Shaft", shaft, { material: "Steel" }),
  part("Connector socket", socket, { material: "Nylon 66" }),
];
