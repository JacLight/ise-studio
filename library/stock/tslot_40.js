// Aluminium T-slot extrusion, 40 series, B-type 8 mm slot (4040, 4080),
// 6063-T5, cut to length.
//
// Source: the common 40-series 8 mm slot profile ("4040 European standard,
// slot 8", sold by the 3D-printer and CNC frame makers; Misumi's HFS8 is
// the 10 mm slot one): 40 mm module, slot opening 8.2 mm, lip 4.3 mm, T cavity 20 wide, slot depth 12.3 mm, core
// bore Ø6.8 (tap drill for M8), hollow corner tubes. Linear mass: published
// 1.5–2.0 kg/m for 4040 by wall weight; the generic section modelled here
// is the heavier one (1.84 kg/m, 4080 3.41 kg/m), and that is the
// catalogue mass.
//
// Stands on its end: the profile in XY, centred on the origin, the length
// along +Z from z = 0. A 4080 is 40 wide in X and 80 deep in Y.

const S40 = TSLOT_SERIES[40];
const cells = Math.min(2, Math.max(1, Math.round(p.cells)));
const L = p.length;
const { section, centres, W, D } = tslotSection(S40, cells);
const bar = section.extrude(L);

const name = `40${40 * cells}`;
const kgPerM = Math.round(section.area * 2.7e-3 * 100) / 100;
stock({
  standard: "40 series B-type, 8 mm slot",
  partNumber: `${name}-B8-${num(L)}`,
  description: `Aluminium T-slot extrusion ${name}, 8 mm slot, 6063-T5 clear anodised, cut to ${num(L)} mm`,
  unitPrice: cents(cells === 1 ? 2.2 + 0.026 * L : 4 + 0.05 * L),
  currency: "USD",
  pack: 1,
  massG: Math.round(kgPerM * L * 10) / 10,
  sources: [
    supplierSearch("Misumi", `aluminum frame ${name} 8mm slot`),
    supplierSearch("McMaster-Carr", "t-slotted framing"),
    supplierSearch("AliExpress", `${name} aluminum extrusion 8mm slot ${num(L)}mm`),
    supplierSearch("Amazon", `${name} aluminum extrusion t slot ${num(L)}mm`),
  ],
});
metric("Length", L, "mm");
metric("Slots", 2 * cells + 2);
metric("Linear mass", kgPerM, "kg/m");

tslotConnectors(W, D, L, centres);
return bar;
