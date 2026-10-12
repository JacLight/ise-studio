// T8 lead screw (Tr8, 2 mm pitch; lead 2, 4 or 8 mm: 1, 2 or 4 starts),
// stainless steel 304, threaded full length, 45° end chamfers.
//
// Sources: ISO 2904 basic profile (see leadscrew.js); the T8 screws sold
// for 3D printers and small CNC (major Ø8, 2 mm pitch; "Tr8×8" 4-start,
// "Tr8×4" 2-start, "Tr8×2" single start), cut lengths 100–500 mm. Mass: the
// screw at its pitch diameter (d2 = 7 mm, which the symmetric profile
// averages to) at 7.93 g/cm³ — about 0.30 g/mm, as sellers list (≈ 95 g
// for 300 mm).
//
// Axis along +Z, end A on z = 0, end B on z = L. Threads are rings by
// default (true volume, exact in STEP); `threads` = 1 sweeps the real
// multi-start helix.

const d = 8;
const pitch = 2;
const lead = p.lead;
const starts = Math.round(lead / pitch);
if (![1, 2, 4].includes(starts)) throw new Error("T8 lead screws come with lead 2, 4 or 8 mm");
const L = p.length;
const R = d / 2;
const depth = trDepth(pitch);
const r = R - depth;

const thread = p.threads >= 0.5
  ? trapezoidHelix({ R, pitch, depth, starts, z0: 0, z1: L })
  : trapezoidRings({ R, pitch, depth, z0: 0, z1: L });
const ch = R + 0.01 - r; // 45° chamfer from the minor diameter out
const envelope = polygon([[0, 0], [r, 0], [R + 0.01, ch], [R + 0.01, L - ch], [r, L], [0, L]]).revolve(360);
const screw = thread.intersect(envelope);

const d2 = d - 0.5 * pitch;
const massG = (Math.PI / 4) * d2 * d2 * L * 7.93e-3;
const size = `Tr8×${lead}${starts > 1 ? ` (P2, ${starts}-start)` : ""}`;
stock({
  standard: "Tr8 (ISO 2904 profile)",
  partNumber: `T8-L${lead}-${num(L)}`,
  description: `Lead screw ${size}, ${num(L)} mm, stainless steel 304`,
  unitPrice: cents(2.5 + 0.012 * L),
  currency: "USD",
  pack: 1,
  massG: Math.round(massG * 10) / 10,
  sources: [
    supplierSearch("Misumi", `lead screw 8mm lead ${lead}`),
    supplierSearch("McMaster-Carr", "acme lead screws"),
    supplierSearch("AliExpress", `T8 lead screw lead ${lead}mm ${num(L)}mm`),
    supplierSearch("Amazon", `T8 lead screw ${num(L)}mm lead ${lead}mm`),
  ],
});
metric("Diameter", d, "mm");
metric("Lead", lead, "mm");
metric("Pitch", pitch, "mm");
metric("Starts", starts);
metric("Minor diameter", 2 * r, "mm");
metric("Length", L, "mm");

connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("End A", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("End B", { at: [0, 0, L], axis: [0, 0, 1] });
return screw;
