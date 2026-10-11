// GT2 idler pulley for 6 mm belt, toothed (20T) or smooth, aluminium body
// running on two shielded ball bearings.
//
// Sources: Gates PowerGrip GT2 manual (see gt2.js); common 3D-printer idler
// dimensions (flanges Ø18 × 1, belt width 6.5, overall 8.5 mm, smooth belt
// surface Ø12); bearings 623ZZ (3×10×4, ISO 15) for the 3 mm bore and
// MR105ZZ (5×10×4) for the 5 mm bore, two side by side; bearing masses
// 623ZZ 1.6 g, MR105ZZ 1.2 g (NMB/SKF catalogues).
//
// Axis along +Z, from z = 0 to 8.5; the belt runs between the flanges
// (z 1…7.5).

//            bearing     Z   Dw      bearing g
const IDLER_BEARING = { 3: ["623ZZ", 7, 1.5875, 1.6], 5: ["MR105ZZ", 8, 1.5875, 1.2] };
const bore = p.bore;
const toothed = p.toothed >= 0.5;
const [bearing, Zb, Dw, bearingMass] = lookup(IDLER_BEARING, bore, "GT2 idler");
const Df = 18;
const W = 8.5;
const seat = 10; // bearing outside diameter
const belt = toothed ? gt2Profile(20).extrude(6.5) : cylinder(6.5, 6);
const body = union(cylinder(1, Df / 2), belt.translate(0, 0, 1), cylinder(1, Df / 2).translate(0, 0, 7.5)).subtract(cylinder(W + 2, seat / 2).translate(0, 0, -1));
const b1 = miniBearing(bore, seat, 4, Zb, Dw).translate(0, 0, 0.25);
const b2 = miniBearing(bore, seat, 4, Zb, Dw).translate(0, 0, 4.25);

// Mass: aluminium body (toothed ring less its 20 grooves of 0.744 mm²) and the two bearings.
const ring = (D) => (Math.PI / 4) * (D * D - seat * seat);
const beltArea = toothed ? ring(gt2Outside(20)) - 20 * 0.744 : ring(12);
const massG = 2.7e-3 * (2 * ring(Df) * 1 + beltArea * 6.5) + 2 * bearingMass;
const kind = toothed ? "toothed" : "smooth";
stock({
  partNumber: `GT2-20T-IDLER-${toothed ? "T" : "S"}-B${num(bore)}-W6`,
  description: `GT2 idler, 20T ${kind}, ${num(bore)} mm bore, for 6 mm belt, 2× ${bearing} bearings`,
  unitPrice: toothed ? 1.5 : 1.3,
  currency: "USD",
  pack: 1,
  massG: Math.round(massG * 100) / 100,
  sources: [
    supplierSearch("AliExpress", `GT2 idler 20T ${kind} bore ${num(bore)}mm`),
    supplierSearch("Amazon", `GT2 20 teeth idler pulley ${num(bore)}mm bore ${kind}`),
    supplierSearch("Misumi", "timing pulley idler GT2"),
  ],
});
metric("Bore", bore, "mm");
metric("Belt surface", toothed ? Math.round(gt2Pitch(20) * 1000) / 1000 : 12, "mm");

connector("Bore", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("Face A", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Face B", { at: [0, 0, W], axis: [0, 0, 1] });
connector("Belt plane", { at: [0, 0, 4.25], axis: [0, 0, 1] });

return [
  part("Idler", body, { material: "Aluminium 6061" }),
  part("Bearing 1", b1, { material: "Bearing steel 52100" }),
  part("Bearing 2", b2, { material: "Bearing steel 52100" }),
];
