// GT2 timing pulley (2 mm pitch) for 6 mm belt: aluminium, one flange, hub
// with two M3 set screws 90° apart.
//
// Sources: Gates PowerGrip GT2 manual (pitch, pitch line differential, see
// gt2.js); common 3D-printer pulley dimensions as sold by Gates-licensed and
// generic makers (20T: flange and hub Ø16, toothed width 7, overall 16 mm;
// 16T: Ø13); set screws ISO 4029 M3×3.
//
// Axis along +Z, hub face on z = 0, hub 0…8 (with the inner flange), teeth
// 8…15, outer flange 15…16.

const PULLEY = { 20: [16, 1.6], 16: [13, 1.5] }; // hub/flange Ø, USD
const teeth = p.teeth;
const bore = p.bore;
const [Dh, price] = lookup(PULLEY, teeth, "GT2 pulley");
const hubL = 8;
const toothW = 7;
const flange = 1;
const H = hubL + toothW + flange;
const OD = gt2Outside(teeth);
const PD = gt2Pitch(teeth);

const hub = cylinder(hubL, Dh / 2);
const toothed = gt2Profile(teeth).extrude(toothW).translate(0, 0, hubL);
const top = cylinder(flange, Dh / 2).translate(0, 0, hubL + toothW);
// Set-screw holes: M3 tapped radially at mid-hub, along +X and +Y.
const zs = hubL / 2 - 0.5;
const tap = internalThreadTool({ d: 3, pitch: 0.5, z0: bore / 2 - 0.5, z1: Dh / 2 + 0.5 });
const toX = (s) => s.rotate(0, 90, 0).translate(0, 0, zs);
const toY = (s) => s.rotate(-90, 0, 0).translate(0, 0, zs);
const body = union(hub, toothed, top).subtract(cylinder(H + 2, bore / 2).translate(0, 0, -1), toX(tap), toY(tap));
const screw = setScrew(3, 3).translate(0, 0, bore / 2);

// Mass from the nominal geometry: aluminium hub, flange and toothed ring
// less its grooves (0.744 mm² each), and two steel M3×3 set screws (0.15 g).
const groove = 0.744;
const ring = (D) => (Math.PI / 4) * (D * D - bore * bore);
const massG = 2.7e-3 * (ring(Dh) * (hubL + flange) + (ring(OD) - teeth * groove) * toothW) + 2 * 0.15;
const size = `${teeth}T bore ${num(bore)}`;
stock({
  partNumber: `GT2-${teeth}T-B${num(bore)}-W6`,
  description: `GT2 timing pulley ${teeth} teeth, ${num(bore)} mm bore, for 6 mm belt, aluminium, 2× M3 set screws`,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG: Math.round(massG * 100) / 100,
  sources: [
    supplierSearch("Misumi", `GT2 timing pulley ${teeth} teeth`),
    supplierSearch("McMaster-Carr", "timing belt pulleys"),
    supplierSearch("AliExpress", `GT2 ${teeth}T pulley bore ${num(bore)}mm 6mm belt`),
    supplierSearch("Amazon", `GT2 ${teeth} teeth pulley ${num(bore)}mm bore`),
  ],
});
metric("Teeth", teeth);
metric("Pitch diameter", Math.round(PD * 1000) / 1000, "mm");
metric("Bore", bore, "mm");

connector("Bore", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("Hub face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Belt plane", { at: [0, 0, hubL + toothW / 2], axis: [0, 0, 1] });
connector("Outer face", { at: [0, 0, H], axis: [0, 0, 1] });

return [
  part("Pulley", body, { material: "Aluminium 6061" }),
  part("Set screw 1", toX(screw), { material: "Steel 45H" }),
  part("Set screw 2", toY(screw), { material: "Steel 45H" }),
];
