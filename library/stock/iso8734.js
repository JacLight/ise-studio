// Parallel pin (dowel), hardened steel, ISO 8734 (DIN 6325), tolerance m6.
//
// Source: ISO 8734:1997 table 1 — nominal diameter d (m6), chamfer c
// (≈ the table's c for type A), the other end rounded with r ≈ d
// (spherical crown), length l overall. Type A (through-hardened 1.3505,
// 550–650 HV).
//
// Stands on its chamfered end at z = 0, axis along +Z, the domed end at z = l.
// Mass: the nominal geometry at 7.85 g/cm³.

const ISO8734 = {
  //   c
  2: 0.35,
  3: 0.5,
  4: 0.63,
  5: 0.8,
  6: 1.2,
  8: 1.6,
};
// Typical price of one pin in a pack of 50 (USD): base + per mm.
const PRICE = { 2: [0.06, 0.002], 3: [0.06, 0.003], 4: [0.07, 0.004], 5: [0.08, 0.005], 6: [0.1, 0.006], 8: [0.15, 0.009] };

const d = p.d;
const L = p.length;
const c = lookup(ISO8734, d, "ISO 8734");
const r = d / 2;
// Chamfered end at the bottom (45°), the domed end at the top: a sphere of
// radius d whose crown is at z = L meets the side at L − (1 − √3/2)·d.
const body = polygon([[0, 0], [r - c, 0], [r, c], [r, L], [0, L]]).revolve(360);
const dome = sphere(d).translate(0, 0, L - d).add(cylinder(L - d, d + 0.5).translate(0, 0, -0.5));
const pin = body.intersect(dome);

// Mass: cylinder, less the chamfer ring, less what the crown takes off.
const crown = (1 - Math.sqrt(3) / 2) * d; // height of the crown's edge below the top
const capVol = (Math.PI * crown * crown * (3 * d - crown)) / 3; // sphere cap of radius d, height crown
const cylinderCut = Math.PI * r * r * crown - capVol; // material between the crown and a flat end
const chamferCut = Math.PI * r * r * c - (Math.PI * c / 3) * (r * r + r * (r - c) + (r - c) * (r - c)); // cylinder less the 45° frustum
const massG = 7.85e-3 * (Math.PI * r * r * L - cylinderCut - chamferCut);
const [base, perMm] = lookup(PRICE, d, "price");
const size = `${num(d)} m6×${num(L)}`;
stock({
  standard: "ISO 8734",
  partNumber: `ISO 8734 ${size} A`,
  description: `Dowel pin Ø${num(d)} m6 × ${num(L)}, hardened steel (type A), one end chamfered, one domed`,
  unitPrice: Math.round((base + perMm * L) * 1000) / 1000,
  currency: "USD",
  pack: 50,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "dowel pins"),
    supplierSearch("Misumi", `dowel pin ${num(d)} ${num(L)} m6 hardened`),
    supplierSearch("AliExpress", `${num(d)}x${num(L)} dowel pin hardened m6`),
  ],
});
metric("Diameter (m6)", d, "mm");

connector("Chamfered end", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Domed end", { at: [0, 0, L], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
return pin;
