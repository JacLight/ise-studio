// Aluminium disc servo horns: the round metal "disc horn" sold for hobby
// servos, a splined hub under a flat disc with tapped holes to bolt a part to.
//
// Sources: the common anodised 6061 disc horns sold for the MG996R / DS3218
// class (25T spline, Ø24–25 mm disc, M3 tapped holes, hub Ø10, ~6.5 mm
// overall) and for the MG90S / SG90 micro class (Ø16 disc, M2 tapped holes,
// ~5.6 mm overall), as listed by AliExpress / Amazon sellers (no maker
// publishes a drawing: hole circles vary by a few tenths between sellers —
// check yours). The spline teeth are the servo's (TowerPro: 25T on Ø5.9 for
// the MG996R, 20T on Ø4.8 for the MG90S).
//
// Stands on its hub: the hub's bottom face at z = 0, the disc's top face at
// z = height. The hub's internal spline is drawn as its major-diameter bore
// (Ø spline + 0.1), so the horn can be shown at any angle on the servo's
// spline without the teeth crossing; the hub sits 0.3 mm above the servo's
// gear-top boss. The centre hole takes the servo's horn screw (M3 for the
// MG996R, M2 for the micro servos), whose head bears on the disc's top face.
// Mass: worked out from this geometry in 6061 (2.70 g/cm³).

const HORNS = {
  //            teeth spline disc  t    hub  height bore  holes pcd  tap centre price servo
  "25T Ø24": [25, 5.9, 24, 2.5, 10, 6.5, 3.5, 4, 16, 3, 3.3, 2.2, "MG996R / MG995 / DS3218"],
  "20T Ø16": [20, 4.8, 16, 2.0, 7, 5.6, 3.2, 4, 11, 2, 2.3, 1.8, "MG90S"],
};

const key = config && HORNS[config] ? config : ["25T Ø24", "20T Ø16"][Math.round(p.model)];
const [teeth, spline, disc, t, hub, height, bore, holes, pcd, tap, centre, price, fits] = lookup(HORNS, key, "servo horn");
const pitch = coarsePitch(tap);

const body = cylinder(height, hub / 2).add(cylinder(t, disc / 2).translate(0, 0, height - t));
const bores = cylinder(bore + 0.01, (spline + 0.1) / 2).translate(0, 0, -0.01).add(cylinder(height + 0.2, centre / 2).translate(0, 0, -0.1));
const taps = [];
for (let i = 0; i < holes; i++) {
  const a = (2 * Math.PI * i) / holes + Math.PI / 4;
  const x = (pcd / 2) * Math.cos(a);
  const y = (pcd / 2) * Math.sin(a);
  taps.push(internalThreadTool({ d: tap, pitch, z0: height - t, z1: height, real: p.threads >= 0.5 }).translate(x, y, 0));
  connector(`Hole ${i + 1}`, { at: [x, y, height], axis: [0, 0, 1] });
}
const horn = body.subtract(bores).subtract(union(taps));

const massG = Math.round(2.7e-3 * horn.volume * 100) / 100;
stock({
  partNumber: `Aluminium disc horn ${teeth}T Ø${num(disc)} M${num(tap)}`,
  description: `Aluminium disc servo horn, ${teeth}-tooth spline for the ${fits}, Ø${num(disc)} mm disc, ${holes} × M${num(tap)} tapped holes on Ø${num(pcd)}`,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("AliExpress", `${teeth}T aluminium servo disc horn ${num(disc)}mm`),
    supplierSearch("Amazon", `${teeth}T metal servo horn round disc ${fits.split(" ")[0]}`),
  ],
});
metric("Spline teeth", teeth);
metric("Tapped thread", tap, "mm");
metric("Tapped depth", t, "mm");
metric("Hole circle", pcd, "mm");

// Where the servo's spline top lands in the hub (hub 0.3 mm above the boss),
// facing the servo; the face a part bolts to.
const visible = bore - 0.1; // the spline's length above the boss
connector("Spline", { at: [0, 0, visible - 0.3], axis: [0, 0, -1] });
connector("Face", { at: [0, 0, height], axis: [0, 0, 1] });
return horn;
