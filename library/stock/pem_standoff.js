// PEM self-clinching standoff, type SO (thru-threaded, steel, zinc): pressed
// into a hole in sheet metal from the back, head flush, it stands a circuit
// board or a panel off the sheet on a threaded barrel.
//
// Source: PennEngineering bulletin SO and distributor listings (SO-M2-4ZI,
// SO-M2-10ZI, SOA-M3-3): hole in sheet 4.22 mm (+0.08) for M2, M2.5 and M3,
// minimum sheet 1.0 mm, max sheet hardness HRB 80, length code = length in
// mm (3–25). Head 4.8 mm across flats and its 1 mm height are the bulletin's
// nominal figures as we read them; the barrel is 4.2 mm, the hole's size.
// Confirm the head on PEM's drawing before tooling.
//
// Geometry: the hex head sits in the sheet, its bottom face flush with the
// sheet's back face at z = −1 (the sheet's top face on z = 0); the round
// barrel stands from z = 0 to z = L. ISO 68-1 thread right through.
// Mass: worked out from this nominal geometry in steel (7.85 g/cm³).

const SO = {
  // thread: hole  barrel  head AF  min edge  price (USD each, packs of 100)
  2: [4.22, 4.2, 4.8, 6.0, 0.22],
  2.5: [4.22, 4.2, 4.8, 6.0, 0.22],
  3: [4.22, 4.2, 4.8, 6.0, 0.2],
};
const LENGTHS = [3, 4, 5, 6, 8, 10, 12, 14, 16, 18, 20, 22, 25];

const d = p.d;
const L = p.length;
const [hole, barrel, af, edge, price] = lookup(SO, d, "PEM SO standoff");
if (!LENGTHS.includes(L)) throw new Error(`PEM SO standoffs come in ${LENGTHS.join(", ")} mm`);
const head = 1.0;
const pitch = coarsePitch(d);
const body = union(hexPrism(af, head).translate(0, 0, -head), cylinder(L, barrel / 2));
const standoff = body.subtract(internalThreadTool({ d, pitch, z0: -head, z1: L, real: p.threads >= 0.5 }));

const d2 = d - 0.6495 * pitch;
const vol = (Math.sqrt(3) / 2) * af * af * head + (Math.PI / 4) * barrel * barrel * L - (Math.PI / 4) * d2 * d2 * (L + head);
const pn = `SO-M${num(d)}-${num(L)}ZI`;
stock({
  standard: "PEM bulletin SO",
  partNumber: pn,
  description: `Self-clinching standoff M${num(d)} × ${num(L)} mm, thru-threaded, zinc-plated steel, for sheet 1 mm and thicker: hole Ø${num(hole)} mm (+0.08), hole centre ${num(edge)} mm from any edge`,
  manufacturer: "PennEngineering (PEM)",
  mpn: pn,
  unitPrice: price,
  currency: "USD",
  pack: 100,
  massG: Math.round(7.85e-3 * vol * 1000) / 1000,
  sources: [supplierSearch("PEM", pn.replace(/ZI$/, "")), supplierSearch("McMaster-Carr", "press-fit standoffs"), supplierSearch("Accu", `self clinching standoff M${num(d)} ${num(L)}mm`)],
});
metric("Hole in sheet Ø", hole, "mm");
metric("Min sheet", 1.0, "mm");
metric("Min hole centre to edge", edge, "mm");
metric("Length", L, "mm");

// The sheet face it stands on, and the top where the board sits (its screw goes in along the axis).
connector("Sheet face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top", { at: [0, 0, L], axis: [0, 0, 1] });
return standoff;
