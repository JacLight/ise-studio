// Linear ball bushing LM…UU (double-sealed), Japanese standard series:
// standard (LMxUU), long (LMxLUU), round-flanged (LMFxUU, LMFxLUU) and
// square-flanged (LMKxUU, LMKxLUU).
//
// Sources: THK LM catalogue, Misumi LMU / LMUW / LMUF / LMUK and Samick
// LM / LM-L / LMF / LMK tables — shaft dr, outside D, length L,
// retaining-ring groove spacing B, groove diameter D1 and width W; ball
// circuits; typical ball sizes; flange Ø Df (square flanges K across flats,
// the Ø Df circle clipped to the square), flange thickness t, bolt circle
// PCD and through holes. Masses checked against the HC-Maschinentechnik LM
// table (LM6UU 7.6 g, LM8UU 15 g, LM10UU 29.5 g, LM12UU 31.5 g, LM16UU 69 g,
// LM20UU 87 g); long ones from Samick LM-L (LM8LUU 31 g, LM10LUU 58 g …);
// a flanged one is its plain bushing plus the flange's steel as modelled
// (LMF8UU 38 g, LMK8UU 30.5 g; sellers list 34–40 g).
//
// Axis along +Z, end A on z = 0, end B on z = L; a flange is at end A (z 0…t),
// its four bolt holes on the PCD at 45°. Parts: the hardened outer sleeve
// (with its retaining-ring grooves, plain sleeves only), the polyacetal
// ball retainer with a slot per ball row, the balls (a loaded row touching
// the shaft and its return row beside it, per circuit) and the two end seals.

//        dr  D   L    B    D1    W   circuits Dw     mass g  USD
const LMUU = {
  6: [12, 19, 13.5, 11.5, 1.1, 4, 1.5875, 7.6, 0.7],
  8: [15, 24, 17.5, 14.3, 1.1, 4, 1.5875, 15, 0.8],
  10: [19, 29, 22.0, 18.0, 1.3, 4, 2.0, 29.5, 1.1],
  12: [21, 30, 23.0, 20.0, 1.3, 4, 2.381, 31.5, 1.2],
  16: [28, 37, 26.5, 27.0, 1.6, 5, 3.969, 69, 2.4],
  20: [32, 42, 30.5, 30.3, 1.6, 5, 3.969, 87, 3.2],
};
//          L    B    mass g USD
const LONG = {
  6: [35, 27, 14.5, 1.6],
  8: [45, 35, 31, 1.7],
  10: [55, 44, 58, 2.3],
  12: [57, 46, 64, 2.6],
  16: [70, 53, 135, 4.6],
  20: [80, 61, 170, 5.8],
};
//          Df   K    t   PCD  hole
const FLANGE = {
  6: [28, 22, 5, 20, 3.5],
  8: [32, 25, 5, 24, 3.5],
  10: [40, 30, 6, 29, 4.5],
  12: [42, 32, 6, 32, 4.5],
  16: [48, 37, 6, 38, 4.5],
  20: [54, 42, 8, 43, 5.5],
};

const dr = p.dr;
const long = p.long >= 0.5;
const form = Math.round(p.flange); // 0 none, 1 round (LMF), 2 square (LMK)
const [D, L0, B0, D1, W, circuits, Dw, mass0, price0] = lookup(LMUU, dr, "LM…UU");
const [L, Bg] = long ? lookup(LONG, dr, "LM…LUU").slice(0, 2) : [L0, B0];
const prefix = ["LM", "LMF", "LMK"][form];
if (prefix === undefined) throw new Error("flange is 0 (none), 1 (round, LMF) or 2 (square, LMK)");
const name = `${prefix}${num(dr)}${long ? "L" : ""}UU`;
const ts = 1.0; // end seal thickness
const rb = dr / 2 + Dw / 2; // loaded balls touch the shaft
const rr = rb + 0.1; // return balls run just clear of it
const rin = rr + Dw / 2 + 0.15; // sleeve bore
const c = 0.3;

// Outer sleeve: retaining-ring grooves on a plain sleeve, else the flange at end A.
const sleeveProfile = [[rin, 0], [D / 2 - c, 0], [D / 2, c]];
if (form === 0) {
  for (const zc of [L / 2 - Bg / 2, L / 2 + Bg / 2]) {
    sleeveProfile.push([D / 2, zc - W / 2], [D1 / 2, zc - W / 2], [D1 / 2, zc + W / 2], [D / 2, zc + W / 2]);
  }
}
sleeveProfile.push([D / 2, L - c], [D / 2 - c, L], [rin, L]);
let sleeve = polygon(sleeveProfile).revolve(360);
let holes = [];
if (form > 0) {
  const [Df, K, t, pcd, hole] = lookup(FLANGE, dr, "flanged LM…UU");
  let outline = circle(Df / 2);
  if (form === 2) outline = outline.intersect(rect(K, K, { center: true }));
  holes = [45, 135, 225, 315].map((a) => [(pcd / 2) * Math.cos(rad(a)), (pcd / 2) * Math.sin(rad(a))]);
  const flange = outline.subtract(circle(rin), ...holes.map(([x, y]) => circle(hole / 2).translate(x, y))).extrude(t);
  sleeve = sleeve.add(flange);
  metric("Flange diameter", form === 2 ? K : Df, "mm");
  metric("Flange thickness", t, "mm");
  metric("Bolt circle", pcd, "mm");
  metric("Bolt hole", hole, "mm");
  connector("Flange face", { at: [0, 0, t], axis: [0, 0, 1] });
  holes.forEach(([x, y], i) => connector(`Hole ${i + 1}`, { at: [x, y, 0], axis: [0, 0, -1] }));
}

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

// Catalogue mass and price: the standard bushing's, the long one's, and a
// flange's steel (its modelled volume) on top.
const [massBase, priceBase] = long ? lookup(LONG, dr, "LM…LUU").slice(2) : [mass0, price0];
let catalogueMass = massBase;
if (form > 0) {
  const [Df, K, t, , hole] = lookup(FLANGE, dr, "flanged LM…UU");
  const area = form === 2 ? circle(Df / 2).intersect(rect(K, K, { center: true })).area : Math.PI * (Df / 2) ** 2;
  catalogueMass += 7.81e-3 * t * (area - Math.PI * (D / 2) ** 2 - Math.PI * (hole / 2) ** 2 * 4);
}
const kind = [`${long ? "long " : ""}linear ball bushing`, `round-flanged ${long ? "long " : ""}linear ball bushing`, `square-flanged ${long ? "long " : ""}linear ball bushing`][form];
stock({
  partNumber: name,
  description: `${kind[0].toUpperCase()}${kind.slice(1)} ${name}, ${num(dr)} mm shaft, ${num(D)}×${num(L)} mm, sealed both ends`,
  mpn: name,
  unitPrice: cents((long ? priceBase : price0) * (form > 0 ? 1.6 : 1)),
  currency: "USD",
  pack: form > 0 || long ? 2 : 4,
  massG: Math.round(catalogueMass * 10) / 10,
  sources: [
    supplierSearch("Misumi", `linear bushing ${["", "round flange ", "square flange "][form]}${long ? "double " : ""}${num(dr)}mm`),
    supplierSearch("McMaster-Carr", form > 0 ? "flanged linear ball bearings" : "linear ball bearings"),
    supplierSearch("AliExpress", `${name} linear bearing`),
    supplierSearch("Amazon", `${name} linear bearing`),
  ],
});
metric("Shaft", dr, "mm");
metric("Outside diameter", D, "mm");
metric("Length", L, "mm");
if (form === 0) metric("Retaining ring groove spacing", Bg, "mm");

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
