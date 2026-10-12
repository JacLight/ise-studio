// O-ring, metric, NBR 70 Shore A (nitrile).
//
// Source: the metric inside diameter × cross-section series (ISO 3601-1
// tolerance class B, the sizes the seal makers stock in 1.5, 2, 2.5 and
// 3 mm sections). Gland for a static radial or face seal from the ISO
// 3601-2 / makers' design tables: groove depth ≈ 0.78 × section (about
// 22 % squeeze), groove width ≈ 1.35 × section (room for the swell), both
// rounded to 0.05 mm.
//
// Lies on its face at z = 0, its section centred at z = cs/2, axis along +Z.
// Mass: this nominal geometry at 1.25 g/cm³ (NBR 70).

const d1 = p.id; // inside diameter
const cs = p.cs; // cross-section
const R = d1 / 2 + cs / 2; // the section's centre
// Finely faceted: a small section on a small ring is all curve.
const ring = circle(cs / 2, { segments: 48 }).translate(R, cs / 2).revolve(360, { segments: Math.max(96, segmentsFor(R + cs / 2)) });

const round05 = (v) => Math.round(v * 20) / 20;
const depth = round05(0.78 * cs);
const width = round05(1.35 * cs);
const massG = 1.25e-3 * ring.volume;
const size = `${num(d1)}×${num(cs)}`;
stock({
  partNumber: `O-ring ${size} NBR70`,
  description: `O-ring ${size} mm (inside Ø × section), NBR 70 Shore A: groove ${num(depth)} deep × ${num(width)} wide`,
  unitPrice: Math.round((0.04 + 0.002 * d1 * cs) * 1000) / 1000,
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 10000) / 10000,
  sources: [
    supplierSearch("McMaster-Carr", "o-rings"),
    supplierSearch("Misumi", `o-ring NBR ${num(d1)}x${num(cs)}`),
    supplierSearch("Amazon", `${num(d1)}x${num(cs)}mm o-ring NBR 70`),
    supplierSearch("AliExpress", `o-ring ${num(d1)}x${num(cs)} NBR`),
  ],
});
metric("Inside Ø", d1, "mm");
metric("Section", cs, "mm");
metric("Outside Ø", d1 + 2 * cs, "mm");
metric("Groove depth", depth, "mm");
metric("Groove width", width, "mm");

connector("Axis", { at: [0, 0, cs / 2], axis: [0, 0, 1] });
connector("Bottom face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top face", { at: [0, 0, cs], axis: [0, 0, 1] });
return ring;
