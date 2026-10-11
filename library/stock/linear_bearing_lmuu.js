// Linear ball bushing LM…UU (double-sealed), Japanese standard series.
//
// Sources: THK LM catalogue and Misumi LMU / NB SM series tables — shaft dr,
// outside D, length L, retaining-ring groove spacing B, groove diameter D1
// and width W; four ball circuits; typical ball sizes. Masses and dims
// checked against the HC-Maschinentechnik LM table (LM6UU 7.6 g, LM8UU
// 15 g, LM10UU 29.5 g, LM12UU 31.5 g; other sellers list 6–8 / 15–16 g).
//
// Axis along +Z, end A on z = 0, end B on z = L. Parts: the hardened outer
// sleeve with its two retaining-ring grooves, the polyacetal ball retainer
// with a slot per ball row, the balls (a loaded row touching the shaft and
// its return row beside it, per circuit) and the two end seals.

//        dr  D   L    B    D1    W   circuits Dw     mass g  USD
const LMUU = {
  6: [12, 19, 13.5, 11.5, 1.1, 4, 1.5875, 7.6, 0.7],
  8: [15, 24, 17.5, 14.3, 1.1, 4, 1.5875, 15, 0.8],
  10: [19, 29, 22.0, 18.0, 1.3, 4, 2.0, 29.5, 1.1],
  12: [21, 30, 23.0, 20.0, 1.3, 4, 2.381, 31.5, 1.2],
};

const dr = p.dr;
const [D, L, Bg, D1, W, circuits, Dw, catalogueMass, price] = lookup(LMUU, dr, "LM…UU");
const name = `LM${num(dr)}UU`;
const ts = 1.0; // end seal thickness
const rb = dr / 2 + Dw / 2; // loaded balls touch the shaft
const rr = rb + 0.1; // return balls run just clear of it
const rin = rr + Dw / 2 + 0.15; // sleeve bore
const c = 0.3;

// Outer sleeve with its retaining-ring grooves.
const sleeveProfile = [[rin, 0], [D / 2 - c, 0], [D / 2, c]];
for (const zc of [L / 2 - Bg / 2, L / 2 + Bg / 2]) {
  sleeveProfile.push([D / 2, zc - W / 2], [D1 / 2, zc - W / 2], [D1 / 2, zc + W / 2], [D / 2, zc + W / 2]);
}
sleeveProfile.push([D / 2, L - c], [D / 2 - c, L], [rin, L]);
const sleeve = polygon(sleeveProfile).revolve(360);

// Balls: each circuit a loaded row and a return row a little round from it.
const z0 = ts + 0.6;
const n = Math.floor((L - 2 * z0) / Dw);
const zStart = L / 2 - (n * Dw) / 2 + Dw / 2;
const balls = [];
const slots = [];
const delta = (1.25 * Dw) / rb;
for (let j = 0; j < circuits; j++) {
  const a0 = (2 * Math.PI * j) / circuits;
  for (const [a, r] of [[a0, rb], [a0 + delta, rr]]) {
    const x = r * Math.cos(a);
    const y = r * Math.sin(a);
    for (let i = 0; i < n; i++) balls.push(sphere(Dw / 2).translate(x, y, zStart + i * Dw));
    slots.push(cylinder(n * Dw + 0.2, Dw / 2 + 0.03).translate(x, y, zStart - Dw / 2 - 0.1));
  }
}
const retainer = polygon([[dr / 2 + 0.3, ts], [rin, ts], [rin, L - ts], [dr / 2 + 0.3, L - ts]]).revolve(360).subtract(union(slots));
const seal = polygon([[dr / 2 + 0.05, 0], [rin, 0], [rin, ts], [dr / 2 + 0.05, ts]]).revolve(360);
const seals = seal.add(seal.translate(0, 0, L - ts));

stock({
  partNumber: name,
  description: `Linear ball bushing ${name}, ${num(dr)} mm shaft, ${num(D)}×${num(L)} mm, sealed both ends`,
  mpn: name,
  unitPrice: price,
  currency: "USD",
  pack: 4,
  massG: catalogueMass,
  sources: [
    supplierSearch("Misumi", `linear bushing ${num(dr)}mm`),
    supplierSearch("McMaster-Carr", "linear ball bearings"),
    supplierSearch("AliExpress", `${name} linear bearing`),
    supplierSearch("Amazon", `${name} linear bearing`),
  ],
});
metric("Shaft", dr, "mm");
metric("Outside diameter", D, "mm");
metric("Length", L, "mm");
metric("Retaining ring groove spacing", Bg, "mm");

connector("Bore", { at: [0, 0, L / 2], axis: [0, 0, 1] });
connector("Outer seat", { at: [D / 2, 0, L / 2], axis: [1, 0, 0], x: [0, 0, 1] });
connector("End A", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("End B", { at: [0, 0, L], axis: [0, 0, 1] });

return [
  part("Sleeve", sleeve, { material: "Bearing steel 52100" }),
  part("Retainer", retainer, { material: "Acetal (POM)" }),
  part("Balls", union(balls), { material: "Bearing steel 52100" }),
  part("Seals", seals, { material: "Nitrile rubber (NBR)" }),
];
