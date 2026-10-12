// GT2 open timing belt (2 mm pitch), 6 mm wide, rubber with glass-fibre
// cords, cut to length from the roll (the belt of a printer's X/Y axes).
//
// Sources: Gates PowerGrip GT2 manual (section and tooth: see gt2.js);
// sold by the metre or in 5 m / 10 m rolls. Mass: the section as modelled
// at 1.35 g/cm³, about 8 g per metre (sellers list 7–9 g/m).
//
// Lies straight along +X from x = 0 to L, 6 mm wide centred on y = 0, the
// land (where the teeth stand) on z = 0: teeth down to z = −0.75, back up
// to z = 0.63, the pitch line at z = 0.254. A tooth centre every 2 mm from
// x = 1.

const L = p.length;
const W = 6;
const teeth = Math.floor(L / 2);
const tooth = gt2ToothPoints(); // u from +0.6 round to −0.6
// One outline in (x, z): the back, then the land back along with each tooth.
const pts = [[0, GT2_BACK], [L, GT2_BACK], [L, 0]];
for (let k = teeth - 1; k >= 0; k--) {
  const xc = 2 * k + 1;
  for (const [u, w] of tooth) pts.push([xc + u, -w]);
}
pts.push([0, 0]);
const section = polygon(pts);
const belt = section.extrude(W).rotate(90, 0, 0).translate(0, W / 2, 0);

const massG = section.area * W * 1.35e-3;
stock({
  standard: "GT2 2 mm",
  partNumber: `GT2-6-OPEN-${num(L)}`,
  description: `GT2 open timing belt, 6 mm wide, cut to ${num(L)} mm (${teeth} teeth), rubber with glass-fibre cords`,
  unitPrice: cents(0.3 + 1.2e-3 * L),
  currency: "USD",
  pack: 1,
  massG: Math.round(massG * 100) / 100,
  sources: [
    supplierSearch("Misumi", "timing belt GT2 2mm open end 6mm"),
    supplierSearch("McMaster-Carr", "timing belts"),
    supplierSearch("AliExpress", "GT2 timing belt 6mm open 5m"),
    supplierSearch("Amazon", "GT2 timing belt 6mm 5 meters"),
  ],
});
metric("Length", L, "mm");
metric("Teeth", teeth);
metric("Width", W, "mm");

connector("End A", { at: [0, 0, GT2_PLD], axis: [-1, 0, 0], x: [0, 0, 1] });
connector("End B", { at: [L, 0, GT2_PLD], axis: [1, 0, 0], x: [0, 0, 1] });
connector("Back", { at: [L / 2, 0, GT2_BACK], axis: [0, 0, 1] });
connector("Teeth", { at: [L / 2, 0, -GT2_TOOTH], axis: [0, 0, -1] });
return belt;
