// Hobby RC servos: TowerPro SG90 (9 g micro, nylon gears), MG90S (micro,
// metal gears), MG996R and MG995 (standard size, metal gears); DSSERVO
// DS3218 (20 kg·cm) and DS3225 (25 kg·cm) digital standard servos, metal
// gears, aluminium middle case.
//
// Sources: TowerPro datasheets — SG90 "22.2 × 11.8 × 31 mm, 9 g"; MG90S
// "22.8 × 12.2 × 28.5 mm, 13.4 g"; MG996R "40.7 × 19.7 × 42.9 mm, 55 g",
// 25-tooth Ø5.9 mm output spline (servodatabase.com: shaft 5.9 mm, 25T);
// MG995 "40.7 × 19.7 × 42.9 mm, 55 g" (the MG996R's case and spline).
// DSSERVO datasheets — DS3218 "40 × 20 × 40.5 mm, 60 g", DS3225
// "40 × 20 × 40.5 mm, 60 g", both 25T output spline, the standard-size
// 49.5 × 10 mm mounting pattern. The DS3218MG drawing (ANNIMOS / DSSERVO
// product datasheet, page 1) dimensions the tabs 54.5 mm overall with holes
// 49.5 × 10 mm apart, the tab underside 24.5 mm above the case bottom and
// the case 37.2 mm tall at the output end; its lid slopes down to about
// 33.8 mm at the far end and is chamfered at the output end (scaled from the
// drawing); the tabs scale to about 3.2 mm thick. Electrical (same sheet):
// stall 19 kg·cm / 1.5 A at 5 V, 21.5 kg·cm / 1.8 A at 6.8 V.
// Tab, hole and shaft positions follow the common dimensioned drawings of
// these servos (standard-size mounting pattern 49.5 × 10 mm with Ø4.4 holes;
// micro servos one Ø2 hole per tab at 27.8 mm): makers' clones differ by a
// few tenths, so check a sample with calipers before printing a tight mount.
//
// The output shaft's axis is the Z axis (x = y = 0), the case bottom on
// z = 0, the case running along +X from the shaft end. The case, tabs and
// gear-top boss are one part whose material lumps what is inside (motor,
// gear train, pot and PCB) at the density that gives the datasheet mass.

const SERVOS = {
  // L, W: case; H: case top; boss: [Ø, top z]; spline: [Ø, top z, teeth, screw Ø];
  // off: shaft axis from the case's shaft end; tab: [overall length, thickness,
  // underside z]; holes: [spacing along X, spacing across Y (0 = one per tab), Ø]
  SG90: { L: 22.5, W: 11.8, H: 22.7, boss: [11.8, 26.7], nub: [5.0, 25.5], spline: [4.8, 31.0, 21, 1.6], off: 5.9, tab: [32.3, 2.5, 15.9], holes: [27.8, 0, 2.0], mass: 9, price: 1.95, gears: "nylon" },
  MG90S: { L: 22.8, W: 12.2, H: 21.6, boss: [12.2, 25.4], nub: [5.0, 24.4], spline: [4.8, 28.5, 20, 1.6], off: 6.0, tab: [32.5, 2.5, 15.6], holes: [27.8, 0, 2.0], mass: 13.4, price: 3.6, gears: "metal" },
  MG996R: { L: 40.7, W: 19.7, H: 36.0, boss: [13.6, 39.5], nub: [9.0, 38.2], spline: [5.9, 42.9, 25, 2.5], off: 10.0, tab: [54.5, 2.5, 26.6], holes: [49.5, 10.0, 4.4], mass: 55, price: 6.5, gears: "metal" },
  MG995: { L: 40.7, W: 19.7, H: 36.0, boss: [13.6, 39.5], nub: [9.0, 38.2], spline: [5.9, 42.9, 25, 2.5], off: 10.0, tab: [54.5, 2.5, 26.6], holes: [49.5, 10.0, 4.4], mass: 55, price: 5.6, gears: "metal" },
  // lid: the sloped case top as [x, z] points (x from the shaft axis, far end first).
  DS3218: { L: 40.0, W: 20.0, H: 33.8, boss: [13.0, 37.2], nub: [8.0, 35.6], spline: [5.9, 40.5, 25, 3.0], off: 10.0, tab: [54.5, 3.2, 24.5], holes: [49.5, 10.0, 4.4], lid: [[30, 33.8], [6.5, 37.2], [-3.7, 37.2], [-10, 34.0]], mass: 60, price: 13.5, gears: "metal", maker: "DSSERVO", torque: 20, stall: [[5, 19, 1.5], [6.8, 21.5, 1.8]] },
  DS3225: { L: 40.0, W: 20.0, H: 33.8, boss: [13.0, 37.2], nub: [8.0, 35.6], spline: [5.9, 40.5, 25, 3.0], off: 10.0, tab: [54.5, 3.2, 24.5], holes: [49.5, 10.0, 4.4], lid: [[30, 33.8], [6.5, 37.2], [-3.7, 37.2], [-10, 34.0]], mass: 60, price: 18.9, gears: "metal", maker: "DSSERVO", torque: 25 },
};
const model = config && SERVOS[config] ? config : ["SG90", "MG90S", "MG996R", "MG995", "DS3218", "DS3225"][Math.round(p.model)];
const s = SERVOS[model];
if (!s) throw new Error(`no servo ${model}`);
const xc = s.L / 2 - s.off; // case centre along X
const [tabL, tabT, tabZ] = s.tab;

// Case with its tabs, the holes through them, and the gear-top boss.
let caseBody = box(s.L, s.W, s.H).translate(-s.off, -s.W / 2, 0);
let tabs = box(tabL, s.W, tabT).translate(xc - tabL / 2, -s.W / 2, tabZ);
const holes = [];
for (const sx of [-1, 1]) {
  for (const sy of s.holes[1] ? [-1, 1] : [0]) holes.push([xc + (sx * s.holes[0]) / 2, (sy * s.holes[1]) / 2]);
}
for (const [x, y] of holes) tabs = tabs.subtract(cylinder(tabT + 0.2, s.holes[2] / 2).translate(x, y, tabZ - 0.1));
// The micro servos' tabs are split from the case by a small groove; the
// standard size has the open slot from each hole to the tab's end.
if (s.holes[1]) {
  for (const [x, y] of holes) {
    const dir = Math.sign(x - xc);
    tabs = tabs.subtract(box(tabL, 1.2, tabT + 0.2).translate(dir > 0 ? x : x - tabL, y - 0.6, tabZ - 0.1));
  }
}
const boss = cylinder(s.boss[1] - s.H + 0.01, s.boss[0] / 2).translate(0, 0, s.H - 0.01);
const nub = cylinder(s.nub[1] - s.H + 0.01, s.nub[0] / 2).translate(s.boss[0] / 2, 0, s.H - 0.01);
// Cable exit: a strain-relief block at the far end, near the bottom.
const relief = box(2.0, 4.6, 2.6).translate(s.L - s.off - 0.01, -2.3, 1.2);
// A sloped lid (drawn in XZ, extruded across the case's width).
const lid = s.lid
  ? polygon([...s.lid, [s.lid[s.lid.length - 1][0], s.H - 0.01], [s.lid[0][0], s.H - 0.01]]).extrude(s.W).rotate(90, 0, 0).translate(0, s.W / 2, 0)
  : null;
const body = lid ? union(caseBody, tabs, boss, nub, relief, lid) : union(caseBody, tabs, boss, nub, relief);

// Output spline: n straight-sided teeth around Ø, the horn screw hole in its top.
const [sd, sTop, teeth, screw] = s.spline;
const ro = sd / 2;
const ri = ro - 0.28;
const pts = [];
for (let i = 0; i < teeth; i++) {
  const a = (2 * Math.PI * i) / teeth;
  const w = Math.PI / teeth;
  pts.push([ri * Math.cos(a - w * 0.55), ri * Math.sin(a - w * 0.55)]);
  pts.push([ro * Math.cos(a - w * 0.3), ro * Math.sin(a - w * 0.3)]);
  pts.push([ro * Math.cos(a + w * 0.3), ro * Math.sin(a + w * 0.3)]);
  pts.push([ri * Math.cos(a + w * 0.55), ri * Math.sin(a + w * 0.55)]);
}
const splineH = sTop - s.boss[1];
const spline = polygon(pts).extrude(splineH + 0.6).translate(0, 0, s.boss[1] - 0.6)
  .subtract(cylinder(Math.min(4, splineH + 0.4), screw / 2).translate(0, 0, sTop - Math.min(4, splineH + 0.4) + 0.01));

// A 30 mm stub of the three-wire lead (brown, red, orange) leaving the relief.
const wire = (y) => cylinder(30, 0.5).rotate(0, 90, 0).translate(s.L - s.off + 1.9, y, 2.5);
const cable = union(wire(-1.0), wire(0), wire(1.0));

const maker = s.maker ?? "TowerPro";
const kind = s.L > 30 ? (s.torque ? `${s.torque} kg·cm digital standard` : "standard") : "micro";
stock({
  partNumber: model,
  description: `${maker} ${model} ${kind} servo, ${s.gears} gears, ${teeth}T output spline`,
  manufacturer: maker,
  mpn: model,
  unitPrice: s.price,
  currency: "USD",
  pack: 1,
  massG: s.mass,
  sources: [
    supplierSearch("Amazon", `${maker} ${model} servo`),
    supplierSearch("AliExpress", `${maker} ${model} servo`),
  ],
});
metric("Spline teeth", teeth);
metric("Spline diameter", sd, "mm");
metric("Hole spacing", s.holes[0], "mm");
metric("Hole spacing across", s.holes[1], "mm");
metric("Tab underside", tabZ, "mm");
metric("Case width", s.W, "mm");
for (const [v, kgcm, amps] of s.stall ?? []) {
  metric(`Stall torque at ${v} V`, Math.round(kgcm * 0.0980665 * 1000) / 1000, "N·m");
  metric(`Stall current at ${v} V`, amps, "A");
}

connector("Output shaft", { at: [0, 0, sTop], axis: [0, 0, 1], part: "Output spline" });
connector("Bottom", { at: [xc, 0, 0], axis: [0, 0, -1], part: "Case" });
connector("Tab seat", { at: [xc, 0, tabZ], axis: [0, 0, -1], part: "Case" });
holes.forEach(([x, y], i) => connector(`Hole ${i + 1}`, { at: [x, y, tabZ + tabT], axis: [0, 0, 1], part: "Case" }));
connector("Cable exit", { at: [s.L - s.off, 0, 2.5], axis: [1, 0, 0], part: "Case" });

return [
  part("Case", body, { material: `${model} body (case, motor, gears, PCB — lumped)` }),
  part("Output spline", spline, { material: s.gears === "metal" ? "Brass" : "POM (acetal)" }),
  part("Cable", cable, { material: "Servo lead (PVC, copper)" }),
];
