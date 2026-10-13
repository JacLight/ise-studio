// PEM self-clinching nut, type CLS (300-series stainless): pressed into a
// punched or laser-cut hole in sheet metal, it gives the sheet a strong
// thread flush with its back face.
//
// Source: PennEngineering bulletin CL, as on the pemnet.com product pages
// (CLS-M2.5-1, CLS-M2-1/-2, CLS-M4-2): hole in sheet (+0.08 mm), C max
// (clinch shank), E ±0.25 (collar), T ±0.25 (collar height), minimum hole
// centre to edge. Shank code 0 / 1 / 2 sets the shank length A (0.77 / 0.97
// / 1.38 mm max) for sheets at least 0.8 / 1.0 / 1.4 mm thick. Max sheet
// hardness HRB 70 (aluminium, mild and galvanised steel; not hardened
// stainless). The M5 row follows the same bulletin table.
//
// Geometry: the round collar of diameter E sits on the sheet's face at z = 0
// (the side it is pressed from) up to z = T; the knurled clinch shank of
// diameter C runs down into the hole to z = −A, with the undercut that the
// displaced sheet flows into. ISO 68-1 thread right through.
// Mass: no maker publishes one; worked out from this nominal geometry in
// 304 stainless (7.93 g/cm³).

const CLS = {
  // thread: hole  C     E     T    min edge  price (USD each, packs of 100)
  2: [4.22, 4.2, 6.35, 1.5, 4.8, 0.16],
  2.5: [4.22, 4.2, 6.35, 1.5, 4.8, 0.16],
  3: [4.22, 4.2, 6.35, 1.5, 4.8, 0.14],
  4: [5.41, 5.38, 7.87, 2.0, 6.9, 0.18],
  5: [6.35, 6.33, 8.64, 2.0, 7.1, 0.22],
};
const SHANK = { 0: [0.77, 0.8], 1: [0.97, 1.0], 2: [1.38, 1.4] }; // A max, min sheet

const d = p.d;
const code = Math.round(p.shank);
const [hole, C, E, T, edge, price] = lookup(CLS, d, "PEM CLS nut");
const [A, minSheet] = lookup(SHANK, code, "shank code");
const pitch = coarsePitch(d);

const collar = cylinder(T, E / 2);
// The clinch shank: a ring of serrations at C, an undercut just under the collar.
const teeth = Math.max(12, Math.round((Math.PI * C) / 0.6));
const star = [];
for (let i = 0; i < 2 * teeth; i++) {
  const a = (Math.PI * i) / teeth;
  const r = i % 2 === 0 ? C / 2 : C / 2 - 0.12;
  star.push([r * Math.cos(a), r * Math.sin(a)]);
}
const undercut = Math.min(0.35 * A, 0.4);
const shank = union(polygon(star).extrude(A - undercut).translate(0, 0, -A), cylinder(undercut, C / 2 - 0.25).translate(0, 0, -undercut));
const nut = collar.add(shank).subtract(internalThreadTool({ d, pitch, z0: -A, z1: T, real: p.threads >= 0.5 }));

const d2 = d - 0.6495 * pitch;
const vol = (Math.PI / 4) * (E * E * T + C * C * A - d2 * d2 * (T + A));
const pn = `CLS-M${num(d)}-${code}`;
stock({
  standard: "PEM bulletin CL",
  partNumber: pn,
  description: `Self-clinching nut M${num(d)}, stainless, for sheet ${num(minSheet)} mm and thicker: hole Ø${num(hole)} mm (+0.08), hole centre ${num(edge)} mm from any edge`,
  manufacturer: "PennEngineering (PEM)",
  mpn: pn,
  unitPrice: price,
  currency: "USD",
  pack: 100,
  massG: Math.round(7.93e-3 * vol * 1000) / 1000,
  sources: [supplierSearch("PEM", pn), supplierSearch("McMaster-Carr", "press-fit nuts"), supplierSearch("Accu", `self clinching nut M${num(d)}`)],
});
metric("Hole in sheet Ø", hole, "mm");
metric("Min sheet", minSheet, "mm");
metric("Min hole centre to edge", edge, "mm");
metric("Collar Ø", E, "mm");

// The face it is pressed against (the sheet's face, nut on top) and the thread's axis.
connector("Sheet face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Thread axis", { at: [0, 0, -A], axis: [0, 0, 1] });
return nut;
