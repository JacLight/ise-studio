// Hexagon nut, style 1, ISO 4032 (DIN 934 sizes), stainless A2-70.
//
// Source: ISO 4032:2012 table 1 — width across flats s (nominal; M10 16,
// M12 18 mm, where DIN 934 had 17/19), nut
// height m (max); bearing face diameter dw (min ≈ 0.9 s, so the 30°
// chamfer starts at 0.95 s across); countersink at the bore to d (max
// 1.0–1.08 d); ISO 261 coarse pitch, ISO 68-1 internal thread.
//
// Stands on its bottom face at z = 0, top face at z = m, axis along +Z.
// Thread: rings by default (exact in STEP); the true helix with `threads`
// on (phase anchored at the bottom face, as examples/fasteners).
// Mass: the ISO nominal geometry at 7.9 g/cm³ (hex prism less the bore at
// the thread's pitch diameter).

const ISO4032 = {
  //    s     m
  2: [4.0, 1.6],
  2.5: [5.0, 2.0],
  3: [5.5, 2.4],
  4: [7.0, 3.2],
  5: [8.0, 4.7],
  6: [10.0, 5.2],
  8: [13.0, 6.8],
  10: [16.0, 8.4],
  12: [18.0, 10.8],
};
// Typical price of one nut in a pack of 100 (stainless A2, USD).
const PRICE = { 2: 0.03, 2.5: 0.03, 3: 0.025, 4: 0.03, 5: 0.035, 6: 0.045, 8: 0.08, 10: 0.13, 12: 0.19 };

const d = p.d;
const [s, m] = lookup(ISO4032, d, "ISO 4032");
const pitch = coarsePitch(d);

// Hex prism with the 30° chamfer on both faces (cones from 0.95 s across).
const r0 = 0.475 * s;
const rise = m; // the cones run the full height; the hex corners clip them
const body = hexPrism(s, m)
  .intersect(cylinder(rise, r0, { top: r0 + rise * Math.tan(rad(60)) }))
  .intersect(cylinder(rise, r0 + rise * Math.tan(rad(60)), { top: r0 }).translate(0, 0, m - rise));
// Bore: the internal thread, with a 90° countersink to d at each face.
const sink = 0.5 * pitch;
const cs = d / 2 + 0.05;
const countersinks = union(
  cylinder(sink + 0.01, cs, { top: cs - sink - 0.01 }).translate(0, 0, -0.01),
  cylinder(sink + 0.01, cs - sink - 0.01, { top: cs }).translate(0, 0, m - sink),
);
const nut = body.subtract(internalThreadTool({ d, pitch, z0: 0, z1: m, real: p.threads >= 0.5 })).subtract(countersinks);

const rho = 7.9e-3;
const d2 = d - 0.6495 * pitch;
const massG = rho * ((Math.sqrt(3) / 2) * s * s - (Math.PI / 4) * d2 * d2) * m;
stock({
  standard: "ISO 4032",
  partNumber: `ISO 4032 M${num(d)} A2-70`,
  description: `Hex nut M${num(d)}, ${num(s)} mm across flats, ${num(m)} mm high, stainless A2-70`,
  unitPrice: lookup(PRICE, d, "price"),
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "hex nuts"),
    supplierSearch("Bolt Depot", `M${num(d)} hex nut stainless`),
    supplierSearch("Misumi", `hex nut M${num(d)} stainless`),
    supplierSearch("AliExpress", `M${num(d)} hex nut DIN 934 A2 stainless`),
  ],
});
metric("Across flats", s, "mm");
metric("Height", m, "mm");

connector("Bottom face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top face", { at: [0, 0, m], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
return nut;
