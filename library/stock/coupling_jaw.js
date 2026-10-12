// Jaw (spider, "plum") shaft coupling: two aluminium hubs with three jaws
// each, interleaved, and a six-lobe polyurethane spider between them.
//
// Sources: the jaw couplings sold for steppers and spindles (generic
// "D20L25" and "D25L30", the pattern of Misumi MJC / Lovejoy L-type with
// straight jaws): D20 — hubs 7.5 long, jaws 9, 1 mm between a jaw tip and
// the other hub, bores 4–8 mm, one M3 set screw per hub; D25 — hubs 9,
// jaws 11, bores 6–12 mm, M4 set screws. Spider 98 Shore A polyurethane,
// its lobes filling the 24° between neighbouring jaws. Mass: the modelled
// parts (sellers list ≈ 15 g for D20L25, ≈ 28 g for D25L30).
//
// Axis along +Z: hub A (bore d1) from z = 0, hub B (bore d2) ending at
// z = L. Hub A's jaws are centred at 0°, 120°, 240°, hub B's at 60°, 180°,
// 300°.

//         hub  jaws gap  spider hole  jaw inner r  set screw USD
const JAW = { 20: [7.5, 9, 1, 4.3, 5.5, 3, 2.6], 25: [9, 11, 1, 6.3, 7.3, 4, 3.8] };
const D = p.D;
const d1 = p.d1;
const d2 = p.d2;
const [Lh, Lj, gap, rs, rj, M, price] = lookup(JAW, D, "jaw coupling");
const L = 2 * Lh + Lj + gap;
const R = D / 2;
if (Math.max(d1, d2) / 2 > rs) throw new Error(`a D${D} jaw coupling takes bores up to ${2 * rs - 0.6} mm`);
const c = 0.4;
const jawDeg = 36; // each jaw's angular width
const clear = 0.4; // degrees between a jaw and a spider lobe

/** An annular sector r0…r1 from a0 to a1 degrees. */
function sector(r0, r1, a0, a1) {
  const n = Math.max(4, Math.ceil((segmentsFor(r1) * (a1 - a0)) / 360));
  const pts = [];
  for (let i = 0; i <= n; i++) pts.push([r1 * Math.cos(rad(a0 + ((a1 - a0) * i) / n)), r1 * Math.sin(rad(a0 + ((a1 - a0) * i) / n))]);
  for (let i = n; i >= 0; i--) pts.push([r0 * Math.cos(rad(a0 + ((a1 - a0) * i) / n)), r0 * Math.sin(rad(a0 + ((a1 - a0) * i) / n))]);
  return polygon(pts);
}

/** A hub with its jaws, built with the hub on z = 0…Lh and the jaws up to Lh + Lj; jaws centred at `a0` + k·120°. */
function hub(bore, a0) {
  const base = polygon([[0, 0], [R - c, 0], [R, c], [R, Lh], [0, Lh]]).revolve(360);
  const jaws = union([0, 120, 240].map((k) => sector(rj, R, a0 + k - jawDeg / 2, a0 + k + jawDeg / 2).extrude(Lj + 0.01).translate(0, 0, Lh - 0.01)));
  // Set screw hole: radial, mid-hub, between two jaws (at a0 + 60°).
  const tool = internalThreadTool({ d: M, pitch: coarsePitch(M), z0: bore / 2 - 0.5, z1: R + 0.5 }).rotate(0, 90, 0).rotate(0, 0, a0 + 60).translate(0, 0, Lh / 2);
  return base.add(jaws).subtract(cylinder(Lh + Lj + 2, bore / 2).translate(0, 0, -1), tool);
}
/** The hub's set screw (cup point at the bore, flush with the outside). */
function setScrewFor(bore, a0) {
  const wall = R - bore / 2;
  const s = { 3: 1.5, 4: 2 }[M];
  return externalThread({ d: M, pitch: coarsePitch(M), z0: 0, z1: wall, tip: "bottom" })
    .subtract(hexSocket(s, 0.6 * wall, wall))
    .translate(0, 0, bore / 2)
    .rotate(0, 90, 0)
    .rotate(0, 0, a0 + 60)
    .translate(0, 0, Lh / 2);
}

const hubA = hub(d1, 0);
// Hub B: built the same way, turned over and set at the top, jaws at 60°.
const flip = (x) => x.mirror(0, 0, 1).translate(0, 0, L);
const hubB = flip(hub(d2, 60));
const screwA = setScrewFor(d1, 0);
const screwB = flip(setScrewFor(d2, 60));

// The spider: a centre ring and six lobes in the gaps between the jaws.
const lobes = [];
for (let k = 0; k < 6; k++) {
  const mid = 30 + 60 * k;
  lobes.push(sector(rj - 0.05, R - 0.4, mid - (60 - jawDeg) / 2 + clear, mid + (60 - jawDeg) / 2 - clear));
}
const spider2D = lobes.reduce((acc, l) => acc.add(l), circle(rj).subtract(circle(rs)));
const spider = spider2D.extrude(Lj - gap).translate(0, 0, Lh + gap);

const massG = (hubA.volume + hubB.volume) * 2.7e-3 + spider.volume * 1.2e-3 + (screwA.volume + screwB.volume) * 7.85e-3;
const name = `D${D}L${L}-${num(d1)}x${num(d2)}`;
stock({
  partNumber: `CPL-JAW-${name}`,
  description: `Jaw (spider) coupling Ø${D}×${L} mm, aluminium hubs, polyurethane spider, bores ${num(d1)} and ${num(d2)} mm, M${M} set screws`,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG: Math.round(massG * 10) / 10,
  sources: [
    supplierSearch("Misumi", `jaw coupling ${num(d1)}mm ${num(d2)}mm`),
    supplierSearch("McMaster-Carr", "jaw couplings"),
    supplierSearch("AliExpress", `plum coupling D${D}L${L} ${num(d1)}x${num(d2)}mm`),
    supplierSearch("Amazon", `jaw spider coupling ${num(d1)}mm to ${num(d2)}mm`),
  ],
});
metric("Bore A", d1, "mm");
metric("Bore B", d2, "mm");
metric("Outside diameter", D, "mm");
metric("Length", L, "mm");
metric("Bore depth", Lh + Lj, "mm");

connector("Bore A", { at: [0, 0, 0], axis: [0, 0, -1], part: "Hub A" });
connector("Bore B", { at: [0, 0, L], axis: [0, 0, 1], part: "Hub B" });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });

return [
  part("Hub A", hubA, { material: "Aluminium 6061" }),
  part("Hub B", hubB, { material: "Aluminium 6061" }),
  part("Spider", spider, { material: "Polyurethane 98A" }),
  part("Set screw A", screwA, { material: "Steel 45H" }),
  part("Set screw B", screwB, { material: "Steel 45H" }),
];
