// Flexible shaft coupling, helical beam type ("D19L25", "D25L30"):
// one-piece aluminium with a helical slot through the wall between the two
// hubs, set screws at each end.
//
// Sources: the generic helical couplings sold for steppers and lead screws
// (D19 × L25 with M3 set screws, bores 4–8 mm; D25 × L30 with M4, bores
// 8–12 mm), after the Ruland / Misumi SCHPW helical beam pattern: a single
// slot about 0.6 mm wide and 3 mm lead over the middle third. Two set
// screws per hub, 90° apart, 3.5 mm in from the end. Mass: the modelled
// aluminium (7075, 2.81 g/cm³) and steel set screws (sellers list 15–17 g for
// D19L25).
//
// Axis along +Z: bore A (d1) from z = 0, bore B (d2) from z = L; each bore
// runs to mid-length. The slot is swept as a true helix (faceted in STEP).

//        L   set screw  slot lead  slot turns  USD
const COUPLING = { 19: [25, 3, 3, 3, 2.2], 25: [30, 4, 3.5, 3, 3.6] };
const D = p.D;
const d1 = p.d1;
const d2 = p.d2;
const [L, M, lead, turns, price] = lookup(COUPLING, D, "flexible coupling");
const R = D / 2;
const c = 0.5;

// Body with the two bores meeting at mid-length.
let body = polygon([[0, c], [c, 0], [R - c, 0], [R, c], [R, L - c], [R - c, L], [0, L]]).revolve(360);
body = body.subtract(cylinder(L / 2 + 0.01, d1 / 2).translate(0, 0, -0.01), cylinder(L / 2 + 0.01, d2 / 2).translate(0, 0, L / 2));

// The helical slot: a thin rectangle swept along a helix through the wall.
const w = 0.6;
const ri = Math.min(d1, d2) / 2 - 0.3;
const ro = R + 0.3;
const z0 = L / 2 - (lead * turns) / 2;
const perTurn = segmentsFor(R);
const steps = turns * perTurn;
const positions = [];
for (let s = 0; s <= steps; s++) {
  const a = (2 * Math.PI * s) / perTurn;
  const zc = z0 + (s / perTurn) * lead;
  for (const [r, dz] of [[ri, -w / 2], [ro, -w / 2], [ro, w / 2], [ri, w / 2]]) positions.push([r * Math.cos(a), r * Math.sin(a), zc + dz]);
}
const at = (s, j) => s * 4 + j;
const triangles = [];
for (let s = 0; s < steps; s++) {
  for (let j = 0; j < 4; j++) {
    const q = (j + 1) % 4;
    triangles.push([at(s, j), at(s + 1, j), at(s + 1, q)], [at(s, j), at(s + 1, q), at(s, q)]);
  }
}
triangles.push([at(0, 0), at(0, 1), at(0, 2)], [at(0, 0), at(0, 2), at(0, 3)]);
triangles.push([at(steps, 0), at(steps, 2), at(steps, 1)], [at(steps, 0), at(steps, 3), at(steps, 2)]);
body = body.subtract(fromMesh(positions, triangles));

// Set screws: M × (wall), cup point at the bore, flush with the outside;
// two per hub along +X and +Y.
const zs = [3.5, L - 3.5];
const bores = [d1, d2];
const screws = [];
const taps = [];
const s = { 3: 1.5, 4: 2 }[M];
zs.forEach((z, h) => {
  const wall = R - bores[h] / 2;
  const tool = internalThreadTool({ d: M, pitch: coarsePitch(M), z0: bores[h] / 2 - 0.5, z1: R + 0.5 });
  const screw = externalThread({ d: M, pitch: coarsePitch(M), z0: 0, z1: wall, tip: "bottom" })
    .subtract(hexSocket(s, 0.6 * wall, wall))
    .translate(0, 0, bores[h] / 2);
  for (const turn of [(x) => x.rotate(0, 90, 0), (x) => x.rotate(-90, 0, 0)]) {
    taps.push(turn(tool).translate(0, 0, z));
    screws.push(turn(screw).translate(0, 0, z));
  }
});
body = body.subtract(union(taps));

const massG = body.volume * 2.81e-3 + union(screws).volume * 7.85e-3;
const name = `D${D}L${L}-${num(d1)}x${num(d2)}`;
stock({
  partNumber: `CPL-HEL-${name}`,
  description: `Flexible shaft coupling, helical beam, Ø${D}×${L} mm aluminium, bores ${num(d1)} and ${num(d2)} mm, M${M} set screws`,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG: Math.round(massG * 10) / 10,
  sources: [
    supplierSearch("Misumi", `helical coupling ${num(d1)}mm ${num(d2)}mm`),
    supplierSearch("McMaster-Carr", "flexible shaft couplings"),
    supplierSearch("AliExpress", `flexible coupling D${D}L${L} ${num(d1)}x${num(d2)}mm`),
    supplierSearch("Amazon", `flexible shaft coupling ${num(d1)}mm to ${num(d2)}mm`),
  ],
});
metric("Bore A", d1, "mm");
metric("Bore B", d2, "mm");
metric("Outside diameter", D, "mm");
metric("Length", L, "mm");
metric("Bore depth", L / 2, "mm");

connector("Bore A", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Bore B", { at: [0, 0, L], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });

return [
  part("Coupling", body, { material: "Aluminium 7075" }),
  part("Set screws", union(screws), { material: "Steel 45H" }),
];
