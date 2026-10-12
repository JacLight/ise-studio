// Aluminium T-slot extrusion, 30 series, B-type 8 mm slot (3030, 3060),
// 6063-T5, cut to length.
//
// Source: 30-series 8 mm slot profile as Misumi HFS6-3030 / HFS6-3060 and
// the common "3030 B-type": 30 mm module, slot opening 8.2 mm, lip 2.2 mm,
// T cavity 16.4 wide, slot depth 9 mm, core bore Ø6.8 (tap drill for M8),
// hollow corner tubes. Linear mass: published 0.85–1.1 kg/m for 3030 by
// maker and wall weight; the generic section modelled here is 0.93 kg/m
// (3060 1.77 kg/m), and that is the catalogue mass.
//
// Stands on its end: the profile in XY, centred on the origin, the length
// along +Z from z = 0. A 3060 is 30 wide in X and 60 deep in Y.

const S30 = TSLOT_SERIES[30];
const cells = Math.min(2, Math.max(1, Math.round(p.cells)));
const L = p.length;
const { section, centres, W, D } = tslotSection(S30, cells);
const bar = section.extrude(L);

const name = `30${30 * cells}`;
const kgPerM = Math.round(section.area * 2.7e-3 * 100) / 100;
stock({
  standard: "30 series B-type, 8 mm slot",
  partNumber: `${name}-B8-${num(L)}`,
  description: `Aluminium T-slot extrusion ${name}, 8 mm slot, 6063-T5 clear anodised, cut to ${num(L)} mm`,
  unitPrice: cents(cells === 1 ? 1.5 + 0.017 * L : 2.6 + 0.032 * L),
  currency: "USD",
  pack: 1,
  massG: Math.round(kgPerM * L * 10) / 10,
  sources: [
    { supplier: "Misumi", sku: `HFS6-${name}-${num(L)}`, url: `https://us.misumi-ec.com/vona2/result/?Keyword=HFS6-${name}-${num(L)}` },
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
