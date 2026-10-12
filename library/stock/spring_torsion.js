// Helical torsion spring, round music wire, close-wound body, two straight
// tangential legs at 90° or 180° to each other in the free position.
//
// Sources: EN 13906-3 (cylindrical helical torsion springs: rate
// R = E d⁴ / (3667 D n) N·mm per degree, n the body coils plus the legs'
// share; the arbor should be about 0.9 × the inside diameter after the coils
// wind down under load); sizes as stocked in metric assortments and by
// Lee Spring / McMaster-Carr (wire × OD, 90° and 180° legs).
//
// Geometry: the body coils are wound close (pitch 1.04 d: drawn a hair open
// so the solid stays manifold); each leg leaves the coil along its tangent,
// level. The legs' free angle sets the coils: n + (180° − angle) / 360° turns
// (whole turns put the legs at 180°, pointing opposite ways). Axis along +Z,
// the bottom of the body on z = 0. Mass: worked out from this geometry at
// 7.85 g/cm³.

const d = p.d;
const OD = p.od;
const n = p.coils;
const angle = p.angle;
const leg = p.leg;
const Dm = OD - d;
const R = Dm / 2;
const pitch = 1.04 * d;
const turns = n + (180 - angle) / 360;
const H = turns * pitch + d; // body height

const helix = coilPath(R, (t) => d / 2 + t * pitch, 0, turns, { perTurn: coilSteps(R) });
// Legs: level, along the coil's tangent where it starts (backwards) and ends.
const a1 = 2 * Math.PI * turns;
const p0 = helix[0];
const p1 = helix[helix.length - 1];
const t0 = [0, -1, 0];
const t1 = [-Math.sin(a1), Math.cos(a1), 0];
const legSteps = 3;
const leg0 = linePath(p0, v3add(p0, v3mul(t0, leg)), legSteps).reverse();
const leg1 = linePath(p1, v3add(p1, v3mul(t1, leg)), legSteps);
const spring = wireAlong([...leg0.slice(0), p0, ...helix.slice(1), ...leg1], d / 2, wireSegments(d));

const rate = (SPRING_E * d ** 4) / (3667 * Dm * (n + (2 * leg) / (3 * Math.PI * Dm)));
const massG = Math.round(7.85e-3 * spring.volume * 1000) / 1000;
const name = `${num(d)}×${num(OD)} ${num(angle)}° ${num(leg)} mm legs`;
stock({
  partNumber: `Torsion spring ${num(d)}×${num(OD)}×${num(n)} ${num(angle)}° L${num(leg)}`,
  description: `Torsion spring, music wire Ø${num(d)} mm, OD ${num(OD)} mm, ${num(turns)} coils, ${num(leg)} mm tangential legs at ${num(angle)}° (free), rate ≈ ${rate.toFixed(2)} N·mm/°`,
  unitPrice: cents(0.3 + 0.004 * d * d * (leg + Dm * n)),
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("McMaster-Carr", "torsion springs"),
    supplierSearch("AliExpress", `torsion spring ${num(d)}mm wire ${num(OD)}mm OD ${num(angle)} degree`),
    supplierSearch("Amazon", `torsion spring assortment ${num(d)}mm`),
  ],
});
metric("Wire diameter", d, "mm");
metric("Outside diameter", OD, "mm");
metric("Inside diameter", OD - 2 * d, "mm");
metric("Body coils", +turns.toFixed(3));
metric("Leg angle", angle, "°");
metric("Leg length", leg, "mm");
metric("Body length", +H.toFixed(3), "mm");
metric("Rate", +rate.toFixed(3), "N·mm/°");
metric("Arbor diameter", +(0.9 * (OD - 2 * d)).toFixed(2), "mm");

connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("Bottom end", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top end", { at: [0, 0, H], axis: [0, 0, 1] });
connector("Leg 1", { at: v3add(p0, v3mul(t0, leg)), axis: t0 });
connector("Leg 2", { at: v3add(p1, v3mul(t1, leg)), axis: t1 });
return spring;
