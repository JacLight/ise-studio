// Linear shaft: induction-hardened, hard-chrome-plated steel rod, h6/g6
// ground, cut to length with 45° end chamfers.
//
// Sources: Misumi SFJ (straight linear shaft) and THK/Hiwin shaft tables —
// diameters 6/8/10/12 mm, end chamfer C0.5 (C0.8 from 10 mm); density of
// CF53 / SUJ2 steel 7.85 g/cm³. Mass is the nominal cylinder.
//
// Axis along +Z, end A on z = 0, end B on z = L.

const d = p.d;
const L = p.length;
const c = d >= 10 ? 0.8 : 0.5;
const rod = polygon([[0, 0], [d / 2 - c, 0], [d / 2, c], [d / 2, L - c], [d / 2 - c, L], [0, L]]).revolve(360);

// Typical price (USD): cut-to-length chrome rod, base plus per millimetre.
const PER_MM = { 6: 0.009, 8: 0.012, 10: 0.016, 12: 0.021 };
const perMm = lookup(PER_MM, d, "linear shaft");
const massG = (Math.PI / 4) * d * d * L * 7.85e-3;
const size = `Ø${num(d)}×${num(L)}`;
stock({
  partNumber: `Linear shaft ${size} h6 hard chrome`,
  description: `Linear shaft ${size} mm, induction hardened, hard chrome plated, h6`,
  unitPrice: cents(1.5 + perMm * L),
  currency: "USD",
  pack: 1,
  massG: Math.round(massG * 10) / 10,
  sources: [
    { supplier: "Misumi", sku: `SFJ${num(d)}-${num(L)}`, url: `https://us.misumi-ec.com/vona2/result/?Keyword=SFJ${num(d)}-${num(L)}` },
    supplierSearch("McMaster-Carr", "linear motion shafts"),
    supplierSearch("AliExpress", `${num(d)}mm linear rod chrome ${num(L)}mm`),
    supplierSearch("Amazon", `${num(d)}mm linear rod ${num(L)}mm`),
  ],
});
metric("Diameter", d, "mm");
metric("Length", L, "mm");

connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("End A", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("End B", { at: [0, 0, L], axis: [0, 0, 1] });
return rod;
