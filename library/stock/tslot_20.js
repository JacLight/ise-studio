// Aluminium T-slot extrusion, 20 series, B-type 6 mm slot (2020, 2040),
// 6063-T5, cut to length.
//
// Source: 20-series 6 mm slot profile as Misumi HFS5-2020 / HFS5-2040 and the
// common "2020 B-type" profile: 20 mm module, slot opening 6.2 mm, lip 1.8 mm,
// slot depth 6.1 mm (T-slot height), centre bore Ø4.2 (tap drill for M5).
// Linear mass: 2020 ≈ 0.50 kg/m (Haluminium 2020 datasheet: slot 6.2,
// T-slot height 6.1, 0.50 kg/m); 2040 six-slot ≈ 1.0 kg/m (published values
// 0.93 kg/m for a four-slot 20×40 (Systeal) to 1.19 kg/m (Framing Tech
// six-slot)). The cavity shape between lip and core varies by maker; the
// modelled section is the generic one, within a few percent of these masses.
//
// Stands on its end: the profile in XY, centred on the origin, the length
// along +Z from z = 0. A 2040 is 20 wide in X and 40 deep in Y.

const SLOT = 6.2; // opening
const LIP = 1.8; // lip thickness
const T_HALF = 5.2; // half width of the T cavity under the lips
const T_DEPTH = 3.6; // depth (from the face) where the T cavity turns to its tapered bottom
const DEPTH = 6.1; // full slot depth from the face
const TIP_HALF = 2.6; // half width of the tapered bottom at the core
const BORE = 4.2;

// One slot cavity on the +Y face of a 20×20 cell whose centre is the origin:
// the mouth through the lips, the T cavity, the tapered bottom.
function slotCavity() {
  const f = 10; // face
  return polygon([
    [-SLOT / 2, f + 0.01],
    [-SLOT / 2, f - LIP],
    [-T_HALF, f - LIP],
    [-T_HALF, f - T_DEPTH],
    [-TIP_HALF, f - DEPTH],
    [TIP_HALF, f - DEPTH],
    [T_HALF, f - T_DEPTH],
    [T_HALF, f - LIP],
    [SLOT / 2, f - LIP],
    [SLOT / 2, f + 0.01],
  ]);
}

const cells = p.cells >= 1.5 ? 2 : 1; // 2020 or 2040
const W = 20;
const D = 20 * cells;
const L = p.length;
const centres = cells === 1 ? [0] : [-10, 10];
let profile = rect(W, D, { center: true });
const cuts = [];
for (const cy of centres) {
  cuts.push(slotCavity().rotate(-90).translate(0, cy)); // +X face
  cuts.push(slotCavity().rotate(90).translate(0, cy)); // −X face
  cuts.push(circle(BORE / 2).translate(0, cy));
}
cuts.push(slotCavity().translate(0, D / 2 - 10)); // +Y end face
cuts.push(slotCavity().rotate(180).translate(0, -(D / 2 - 10))); // −Y end face
if (cells === 2) {
  // Between the two cells: the two inner slot cavities joined into one
  // closed void, as a 2040 has.
  cuts.push(
    polygon([
      [-TIP_HALF, -DEPTH],
      [TIP_HALF, -DEPTH],
      [T_HALF, -T_DEPTH],
      [T_HALF, T_DEPTH],
      [TIP_HALF, DEPTH],
      [-TIP_HALF, DEPTH],
      [-T_HALF, T_DEPTH],
      [-T_HALF, -T_DEPTH],
    ]),
  );
}
profile = profile.subtract(...cuts);
const bar = profile.extrude(L);

const name = cells === 1 ? "2020" : "2040";
const kgPerM = cells === 1 ? 0.5 : 1.0;
const price = cells === 1 ? 0.9 + 0.009 * L : 1.4 + 0.016 * L;
stock({
  standard: "20 series B-type, 6 mm slot",
  partNumber: `${name}-B6-${num(L)}`,
  description: `Aluminium T-slot extrusion ${name}, 6 mm slot, 6063-T5 clear anodised, cut to ${num(L)} mm`,
  unitPrice: cents(price),
  currency: "USD",
  pack: 1,
  massG: Math.round(kgPerM * L * 10) / 10,
  sources: [
    { supplier: "Misumi", sku: `HFS5-${name}-${num(L)}`, url: `https://us.misumi-ec.com/vona2/result/?Keyword=HFS5-${name}-${num(L)}` },
    supplierSearch("McMaster-Carr", "t-slotted framing"),
    supplierSearch("AliExpress", `${name} aluminum extrusion 6mm slot ${num(L)}mm`),
    supplierSearch("Amazon", `${name} aluminum extrusion t slot ${num(L)}mm`),
  ],
});
metric("Length", L, "mm");
metric("Slots", 2 * cells + 2);

// Ends, then each slot line at its mouth, mid-length, facing out.
connector("End A", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("End B", { at: [0, 0, L], axis: [0, 0, 1] });
centres.forEach((cy, i) => {
  const tag = cells === 1 ? "" : ` ${i + 1}`;
  connector(`Slot +X${tag}`, { at: [W / 2, cy, L / 2], axis: [1, 0, 0], x: [0, 0, 1] });
  connector(`Slot -X${tag}`, { at: [-W / 2, cy, L / 2], axis: [-1, 0, 0], x: [0, 0, 1] });
});
connector("Slot +Y", { at: [0, D / 2, L / 2], axis: [0, 1, 0], x: [0, 0, 1] });
connector("Slot -Y", { at: [0, -D / 2, L / 2], axis: [0, -1, 0], x: [0, 0, 1] });
return bar;
