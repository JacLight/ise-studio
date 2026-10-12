// Flanged deep groove ball bearing (F…, MF…), single row, rubber sealed
// (-2RS) or steel shielded (ZZ) both sides.
//
// Sources: boundary dimensions d × D × B and flange Df × Bf from the
// MinebeaMitsumi (NMB) miniature & instrument catalogue and the SKF / EZO
// flanged miniature tables (F623 3×10×4, flange 11.5×1; F624 4×13×5, 15×1;
// F625 5×16×5, 18×1; F626 6×19×6, 22×1.5; F608 8×22×7, 25×1.5; F688
// 8×16×5, 18×1.1; F695 5×13×4, 15×1; MF63 3×6×2.5, 7.2×0.6; MF85 5×8×2.5,
// 9.2×0.6; MF105 5×10×4, 11.5×1; MF128 8×12×3.5, 13.6×0.8). Ball sets as
// the unflanged bearing of the same size (bearing_deep_groove). Masses: the
// unflanged bearing's catalogue mass plus the steel flange ring (NMB lists
// F623 1.7 g, F608 13 g).
//
// Axis +Z, face A (the flanged face) on z = 0, face B on z = B; the flange
// runs from z = 0 to Bf, so the housing face it seats on is at z = Bf.

//                d  D   B    Df    Bf   balls  Dw     mass g USD
const FLANGED = {
  "3x10x4": ["F623", 11.5, 1.0, 7, 1.5875, 1.7, 0.8],
  "4x13x5": ["F624", 15, 1.0, 7, 2.381, 3.4, 0.8],
  "5x16x5": ["F625", 18, 1.0, 7, 2.778, 5.3, 0.9],
  "6x19x6": ["F626", 22, 1.5, 7, 3.175, 9.4, 1.0],
  "8x22x7": ["F608", 25, 1.5, 7, 3.969, 13, 1.0],
  "8x16x5": ["F688", 18, 1.1, 10, 2.381, 4.1, 1.1],
  "5x13x4": ["F695", 15, 1.0, 8, 2.0, 2.8, 0.9],
  "3x6x2.5": ["MF63", 7.2, 0.6, 8, 0.794, 0.33, 1.0],
  "5x8x2.5": ["MF85", 9.2, 0.6, 11, 0.794, 0.46, 1.0],
  "5x10x4": ["MF105", 11.5, 1.0, 8, 1.5875, 1.4, 0.9],
  "8x12x3.5": ["MF128", 13.6, 0.8, 12, 1.0, 1.4, 1.1],
};

const d = p.d;
const D = p.D;
const B = p.B;
const shielded = p.shield >= 0.5;
const key = `${num(d)}x${num(D)}x${num(B)}`;
const [series, Df, Bf, Z, Dw, catalogueMass, price] = lookup(FLANGED, key, "flanged ball bearing");
const closure = shielded ? "ZZ" : "2RS";
const name = shielded ? `${series}ZZ` : `${series}-2RS`;

const { inner, outer, balls, cage, closures } = radialBallBearing({ d, D, B, Z, Dw, closure, flange: [Df, Bf] });

stock({
  standard: "ISO 15 (flanged)",
  partNumber: name,
  description: `Flanged ball bearing ${name}, ${num(d)}×${num(D)}×${num(B)} mm, flange Ø${num(Df)}×${num(Bf)}, ${shielded ? "metal shields" : "rubber sealed"} both sides`,
  mpn: name,
  unitPrice: cents(price + (shielded ? -0.1 : 0)),
  currency: "USD",
  pack: 10,
  // Shields weigh what seals do to within a few hundredths of a gram.
  massG: catalogueMass,
  sources: [
    supplierSearch("VXB", `${name} flanged bearing`),
    supplierSearch("McMaster-Carr", "flanged ball bearings"),
    supplierSearch("Misumi", `flanged bearing ${series}`),
    supplierSearch("AliExpress", `${name} flange bearing ${num(d)}x${num(D)}x${num(B)}`),
    supplierSearch("Amazon", `${name} flanged ball bearing`),
  ],
});
metric("Bore", d, "mm");
metric("Outside diameter", D, "mm");
metric("Width", B, "mm");
metric("Flange diameter", Df, "mm");
metric("Flange width", Bf, "mm");
metric("Balls", Z);

bearingConnectors(D, B);
connector("Flange seat", { at: [0, 0, Bf], axis: [0, 0, 1] });

return [
  part("Inner ring", inner, { material: "Bearing steel 52100" }),
  part("Outer ring", outer, { material: "Bearing steel 52100" }),
  part("Balls", balls, { material: "Bearing steel 52100" }),
  part("Cage", cage, { material: "Steel (cage)" }),
  part(shielded ? "Shields" : "Seals", closures, { material: shielded ? "Steel (shield)" : "Nitrile rubber (NBR)" }),
];
