// Brass heat-set threaded insert for 3D-printed parts (Ruthex RX and CNC
// Kitchen sizes).
//
// Source: the makers' dimension tables (as published with the Ruthex RX-M2x4,
// RX-M3x5.7, RX-M4x8.1, RX-M5x9.5, RX-M6x12.7 and CNC Kitchen M2×3, M2.5×4,
// M3×3, M4×4 and M5×5.8 "short", M3×4 "Voron" listings, e.g. 3djake.com;
// the short inserts share their long sibling's diameters): D1 knurled outside diameter,
// D2 lead-in (tip) diameter, D3 recommended printed hole diameter, L length,
// W minimum wall around the hole. Recommended hole depth: L + 1 mm, so the
// plastic the insert displaces has room below it.
//
// Geometry: two bands of straight knurl (ribs between D1 and D1 − 0.6 mm)
// separated by a groove at D2 that locks against pull-out, a lead-in taper
// from D2 at the tip, and the ISO 68-1 internal thread right through.
// Seated flush: the top face at z = 0 (the printed surface), the tip at z = −L.
// Mass: no maker publishes one; worked out from this nominal geometry in
// brass CuZn39Pb3 (8.47 g/cm³).

const INSERTS = {
  // key "d×L":  D1    D2    D3    W   maker            part number      price (USD, each in packs of 100)
  "2×3": [3.6, 3.1, 3.2, 1.3, "CNC Kitchen", "M2x3", 0.05],
  "2×4": [3.6, 3.1, 3.2, 1.3, "Ruthex", "RX-M2x4", 0.05],
  "2.5×4": [4.6, 3.9, 4.0, 1.6, "CNC Kitchen", "M2.5x4", 0.06],
  "3×3": [4.6, 3.9, 4.0, 1.6, "CNC Kitchen", "M3x3 short", 0.05],
  "3×4": [5.0, 4.25, 4.4, 1.3, "CNC Kitchen", "M3x5x4 Voron", 0.06],
  "3×5.7": [4.6, 3.9, 4.0, 1.6, "Ruthex", "RX-M3x5.7", 0.06],
  "4×4": [6.3, 5.5, 5.6, 2.1, "CNC Kitchen", "M4x4 short", 0.08],
  "4×8.1": [6.3, 5.5, 5.6, 2.1, "Ruthex", "RX-M4x8.1", 0.09],
  "5×5.8": [7.1, 6.3, 6.4, 2.6, "CNC Kitchen", "M5x5.8 short", 0.1],
  "5×9.5": [7.1, 6.3, 6.4, 2.6, "Ruthex", "RX-M5x9.5", 0.12],
  "6×12.7": [8.7, 7.9, 8.0, 3.0, "Ruthex", "RX-M6x12.7", 0.2],
};

const d = p.d;
const L = p.length;
const [D1, D2, D3, W, maker, mpn, price] = lookup(INSERTS, `${num(d)}×${num(L)}`, "heat-set insert");
const pitch = coarsePitch(d);

// Knurl: a star of ribs, one every ~0.7 mm of circumference.
const depth = 0.3;
const teeth = Math.max(12, Math.round((Math.PI * D1) / 0.7));
const star = [];
for (let i = 0; i < 2 * teeth; i++) {
  const a = (Math.PI * i) / teeth;
  const r = i % 2 === 0 ? D1 / 2 : D1 / 2 - depth;
  star.push([r * Math.cos(a), r * Math.sin(a)]);
}
const knurl = polygon(star);
const lead = 0.15 * L; // lead-in taper at the tip
const groove = 0.14 * L; // locking groove between the bands
const mid = -L + lead + (L - lead - groove) / 2; // groove's bottom edge
const bandA = knurl.extrude(mid - (-L + lead)).translate(0, 0, -L + lead);
const bandB = knurl.extrude(-(mid + groove)).translate(0, 0, mid + groove);
// Core at D2 the full length, the tip's taper from D2 out to the knurl.
const core = cylinder(L, D2 / 2).translate(0, 0, -L);
const taper = knurl.extrude(lead).translate(0, 0, -L).intersect(cylinder(lead, D2 / 2, { top: D1 / 2 }).translate(0, 0, -L));
const body = union(core, bandA, bandB, taper);
const insert = body.subtract(internalThreadTool({ d, pitch, z0: -L, z1: 0, real: p.threads >= 0.5 }));

// Mass from this geometry: star area × band lengths, the taper as the mean
// of core and knurl, the core in the groove, less the bore at the thread's
// pitch diameter.
const starArea = teeth * (D1 / 2) * (D1 / 2 - depth) * Math.sin(Math.PI / teeth);
const coreArea = (Math.PI / 4) * D2 * D2;
const d2 = d - 0.6495 * pitch;
const vol = starArea * (L - lead - groove) + coreArea * groove + ((starArea + coreArea) / 2) * lead - (Math.PI / 4) * d2 * d2 * L;
const massG = 8.47e-3 * vol;
const depthHole = L + 1;
const size = `M${num(d)}×${num(L)}`;
stock({
  partNumber: `${maker} ${mpn}`,
  description: `Heat-set insert ${size}, brass, knurled Ø${num(D1)}: print a Ø${num(D3)} mm hole ${num(depthHole)} mm deep, wall at least ${num(W)} mm`,
  manufacturer: maker,
  mpn,
  unitPrice: price,
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch(maker === "Ruthex" ? "Ruthex" : "CNC Kitchen", `${mpn}`),
    supplierSearch("Amazon", `${maker} threaded insert ${mpn}`),
    supplierSearch("AliExpress", `M${num(d)} heat set insert brass ${num(L)}mm`),
  ],
});
metric("Printed hole Ø", D3, "mm");
metric("Hole depth", depthHole, "mm");
metric("Minimum wall", W, "mm");
metric("Outside Ø", D1, "mm");

// The hole's axis (from the tip, up through the top) and the top face that
// sits flush with the print.
connector("Top face", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("Hole axis", { at: [0, 0, -L], axis: [0, 0, 1] });
return insert;
