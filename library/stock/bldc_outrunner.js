// Brushless outrunner motors for drones and RC planes: the stator (named by
// its diameter × height in mm, "2212") inside a spinning bell of magnets,
// the mounting base below with M3 holes, three phase leads.
//
// Sources: EMAX MT2204 II 2300KV (bell Ø27.9 × 18.8 mm body, M5 prop
// shaft, 16 × 19 mm M3 base pattern, 25 g); the A2212/13T 1000KV
// (DYS / Turnigy pattern: bell Ø27.7, 26.0 mm body, Ø3.17 mm shaft,
// 16 / 19 mm M3 cross pattern, 47 g); 2810-class 7" FPV motors (e.g. iFlight
// XING 2810 / T-Motor F100 class: bell Ø34.0 × 22.5 mm body, M5 prop shaft,
// 19 × 19 mm M3 pattern, 67 g). Bell diameters and heights vary by maker by
// a few tenths: check the one you buy before printing a tight mount.
//
// The base's mounting face on z = 0, the motor up +Z, the shaft on the Z
// axis. Bell and base are aluminium, the magnet ring NdFeB; the stator
// (laminations, copper windings, bearings) is one lumped part whose density
// gives the datasheet mass.

const MOTORS = {
  // bell Ø, body height, shaft [Ø, length above the bell, thread M (0 = plain)],
  // holes: [[spacing, angle°]…] (pairs across the centre), kv, mass g, price, model
  2204: { bell: 27.9, H: 18.8, base: 2, shaft: [5, 11, 5], holes: [[16, 0], [19, 90]], kv: 2300, mass: 25, price: 14.9, model: "EMAX MT2204 II 2300KV" },
  2212: { bell: 27.7, H: 26.0, shaft: [3.17, 13, 0], holes: [[16, 0], [19, 90]], kv: 1000, mass: 47, price: 7.9, model: "A2212/13T 1000KV" },
  2810: { bell: 34.0, H: 22.5, shaft: [5, 13, 5], holes: [[19 * Math.SQRT2, 45], [19 * Math.SQRT2, 135]], kv: 1100, mass: 67, price: 24.9, model: "2810 1100KV (7\" FPV)" },
};
const size = Math.round(p.size);
const m = lookup(MOTORS, size, "outrunner size");
const R = m.bell / 2;
const base = m.base ?? 3; // base thickness
const gap = 0.6; // between the base and the spinning bell
const z0 = base + gap;

// Base: a cross of arms round a hub, the M3 holes tapped through the arms,
// the bearing tube rising into the stator.
const holes = [];
for (const [spacing, deg] of m.holes) {
  for (const s of [-1, 1]) {
    const a = rad(deg);
    holes.push([(s * spacing * Math.cos(a)) / 2, (s * spacing * Math.sin(a)) / 2]);
  }
}
const reach = Math.max(...holes.map(([x, y]) => Math.hypot(x, y))) + 3.2;
let baseSolid = cylinder(base, Math.min(R - 1, reach - 1));
for (const [x, y] of holes) {
  const a = Math.atan2(y, x);
  const arm = box(reach, 6.4, base).translate(0, -3.2, 0).rotate(0, 0, (a * 180) / Math.PI);
  baseSolid = baseSolid.add(arm).add(cylinder(base, 3.2).translate(x, y, 0));
}
for (const [x, y] of holes) baseSolid = baseSolid.subtract(internalThreadTool({ d: 3, pitch: 0.5, z0: 0, z1: base }).translate(x, y, 0));
baseSolid = baseSolid.subtract(cylinder(base + 0.2, m.shaft[0] / 2 + 1.2).translate(0, 0, -0.1)); // circlip / shaft end recess
// Stator: the size names it — Ø22 × 12 mm for a 2212 — a 12-slot drum of
// laminations and windings near the bell's top, on the base's hub tube.
const statorR = Math.floor(size / 100) / 2;
const statorH = size % 100;
const sTop = base + m.H - 1.5 - 1.0;
const sBot = sTop - statorH;
const hubR = 0.32 * statorR;
const tube = cylinder(sTop - base + 0.01, hubR).translate(0, 0, base - 0.01)
  .add(cylinder(sBot - base, hubR + 1.5).translate(0, 0, base - 0.01))
  .subtract(cylinder(m.H, m.shaft[0] / 2 + 0.3).translate(0, 0, base - 0.5));
const teeth = 12;
let stator = cylinder(statorH, statorR).translate(0, 0, sBot).subtract(cylinder(statorH + 1, hubR).translate(0, 0, sBot - 0.5));
for (let i = 0; i < teeth; i++) {
  stator = stator.subtract(box(statorR * 0.45, 1.6, statorH + 1).translate(statorR * 0.62, -0.8, sBot - 0.5).rotate(0, 0, (360 * (i + 0.5)) / teeth));
}

// Bell: a can open at the bottom, its top with six windows round the hub,
// the shaft through its centre; the ring of magnets inside it round the
// stator, 0.35 mm air gap.
const wall = 0.9;
let bell = cylinder(m.H - gap, R).translate(0, 0, z0).subtract(cylinder(m.H - gap - 1.5, R - wall).translate(0, 0, z0 - 0.01));
for (let i = 0; i < 6; i++) {
  bell = bell.subtract(cylinder(2, (R - wall) * 0.22).translate((R - wall) * 0.6, 0, base + m.H - 2).rotate(0, 0, 60 * i));
}
const top = base + m.H + 2.5;
bell = bell.add(cylinder(2.5, m.shaft[0] / 2 + 2.2).translate(0, 0, base + m.H - 0.01))
  .subtract(cylinder(m.H, m.shaft[0] / 2).translate(0, 0, top - m.H + 0.1));
const magnets = cylinder(statorH + 1, R - wall + 0.01).subtract(cylinder(statorH + 2, statorR + 0.35).translate(0, 0, -0.5)).translate(0, 0, sBot - 0.5);

// Shaft: through the motor from the base's recess up past the bell; a
// prop thread (ISO rings) on M5 shafts.
const [sd, above, thread] = m.shaft;
const tip = top + above;
let shaft = cylinder(tip - 1 - (thread ? 9 : 0), sd / 2).translate(0, 0, 1);
if (thread) shaft = shaft.add(externalThread({ d: thread, pitch: 0.8, z0: tip - 9 - 0.01, z1: tip, tip: "top" }));

// Three phase leads out of the base's side.
const lead = (y) => cylinder(30, 0.8).rotate(0, 90, 0).translate(R - 4, y, base / 2);
const leads = union(lead(-2), lead(0), lead(2));

stock({
  partNumber: `BLDC ${size} ${m.kv}KV`,
  description: `Brushless outrunner motor ${size} ${m.kv}KV (typ. ${m.model}), bell Ø${num(m.bell)} mm, ${thread ? `M${thread} prop shaft` : `Ø${num(sd)} mm shaft`}`,
  unitPrice: m.price,
  currency: "USD",
  pack: 1,
  massG: m.mass,
  sources: [
    supplierSearch("AliExpress", `${size} brushless motor ${m.kv}KV`),
    supplierSearch("Amazon", `${m.model} brushless motor`),
  ],
});
metric("Stator size", size);
metric("KV", m.kv, "rpm/V");
metric("Bell diameter", m.bell, "mm");
metric("Shaft diameter", sd, "mm");

connector("Shaft", { at: [0, 0, top], axis: [0, 0, 1], part: "Shaft" });
connector("Prop seat", { at: [0, 0, top], axis: [0, 0, 1], part: "Bell" });
connector("Base", { at: [0, 0, 0], axis: [0, 0, -1], part: "Base" });
holes.forEach(([x, y], i) => connector(`Hole ${i + 1}`, { at: [x, y, 0], axis: [0, 0, -1], part: "Base" }));

return [
  part("Base", union(baseSolid, tube), { material: "Aluminium" }),
  part("Stator", stator, { material: `${size} stator, laminations and windings (lumped)` }),
  part("Bell", bell, { material: "Aluminium" }),
  part("Magnets", magnets, { material: "NdFeB" }),
  part("Shaft", shaft, { material: "Steel" }),
  part("Leads", leads, { material: "Silicone wire (copper)" }),
];
