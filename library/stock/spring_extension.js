// Helical extension (tension) spring, round music wire, close-wound body,
// a full machine loop at each end in the same plane.
//
// Sources: EN 13906-2 (cylindrical helical extension springs: rate
// R = G d⁴ / (8 D³ n), all body coils active, initial tension from winding
// close); end forms per the German loop ("deutsche Öse", full loop, inside
// diameter ≈ the body's inside diameter); sizes as stocked in metric
// assortments and by Lee Spring / McMaster-Carr (wire × OD × overall
// length, e.g. 0.8 × 6 × 30 mm, 1.0 × 8 × 40 mm).
//
// Geometry: the body coils are wound close (pitch 1.04 d: drawn a hair open
// so the solid stays manifold); at each end the last coil bends up through a
// radius of 1.5 d and closes into a full loop of the body's mean diameter
// in a plane through the axis. Axis along +Z, the bottom loop's outside on
// z = 0, the top loop's outside on z = L (L the overall length). Mass:
// worked out from this geometry at 7.85 g/cm³.

const d = p.d;
const OD = p.od;
const L = p.length;
const Dm = OD - d;
const R = Dm / 2;
const bend = 1.5 * d; // bend radius of the loop's riser (centre line)
// Overall: body + two loops; each loop's centre line reaches rise + 2R above
// the body's end coil, plus half a wire outside. The loops' rise (at least
// 1.5 d) takes up what whole coils leave of the length.
const pitch = 1.04 * d;
const n = Math.floor((L - 2 * (1.5 * d + 2 * R + d / 2)) / pitch);
if (n < 3) throw new Error(`extension spring: ${num(L)} mm overall leaves room for ${n} coils of OD ${num(OD)}`);
if (R < 2.2 * d) throw new Error(`extension spring: spring index ${(Dm / d).toFixed(1)} is too small for a full loop`);
const body = n * pitch;
const rise = (L - body - 4 * R - d) / 2; // loop's lowest point above the body's end coil (centre line)
const z0 = rise + 2 * R + d / 2; // the body's bottom coil (centre line)

// The top end: from the end of the body helix (at angle 0, heading +Y),
// bend up through `bend`, rise, then round the loop in the plane y = bend.
function loopEnd(zTop) {
  const pts = [];
  const steps = 8;
  for (let s = 1; s <= steps; s++) {
    const a = ((Math.PI / 2) * s) / steps;
    pts.push([R, bend * Math.sin(a), zTop + bend * (1 - Math.cos(a))]);
  }
  const zc = zTop + rise + R;
  pts.push(...linePath(pts[pts.length - 1], [R, bend, zc], 2));
  const per = coilSteps(R);
  const end = (300 / 360) * per;
  for (let s = 1; s <= end; s++) {
    const a = (2 * Math.PI * s) / per;
    pts.push([R * Math.cos(a), bend, zc + R * Math.sin(a)]);
  }
  return pts;
}
const helix = coilPath(R, (t) => z0 + t * pitch, 0, n, { perTurn: coilSteps(R) });
const top = loopEnd(z0 + body);
// The bottom end is the top end turned half round the X axis about the body's middle.
const zm = z0 + body / 2;
const bottom = loopEnd(z0 + body).map(([x, y, z]) => [x, -y, 2 * zm - z]).reverse();
const spring = wireAlong([...bottom, ...helix, ...top], d / 2, wireSegments(d));

const rate = (SPRING_G * d ** 4) / (8 * Dm ** 3 * n);
const massG = Math.round(7.85e-3 * spring.volume * 1000) / 1000;
const name = `${num(d)}×${num(OD)}×${num(L)}`;
const inside = L - 2 * d;
stock({
  partNumber: `Extension spring ${name}`,
  description: `Extension spring, music wire Ø${num(d)} mm, OD ${num(OD)} mm, ${num(L)} mm overall (${num(inside)} mm inside the loops), ${n} body coils, full loops, rate ≈ ${rate.toFixed(3)} N/mm`,
  unitPrice: cents(0.22 + 0.002 * d * d * L),
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("McMaster-Carr", "extension springs"),
    supplierSearch("AliExpress", `extension spring ${num(d)}mm wire ${num(OD)}mm OD ${num(L)}mm`),
    supplierSearch("Amazon", `extension spring assortment ${num(d)}mm`),
  ],
});
metric("Wire diameter", d, "mm");
metric("Outside diameter", OD, "mm");
metric("Overall length", L, "mm");
metric("Length inside loops", +inside.toFixed(3), "mm");
metric("Body length", +(body + d).toFixed(2), "mm");
metric("Body coils", n);
metric("Rate", +rate.toFixed(4), "N/mm");

// Where a pin or hook through each loop bears (the loop's inside, farthest
// from the body), pulling outward; and the loops' centres, axis across the
// loop for a pin through it.
connector("Bottom end", { at: [0, -bend, d], axis: [0, 0, -1] });
connector("Top end", { at: [0, bend, L - d], axis: [0, 0, 1] });
connector("Bottom loop", { at: [0, -bend, d / 2 + R], axis: [0, -1, 0], x: [0, 0, -1] });
connector("Top loop", { at: [0, bend, L - d / 2 - R], axis: [0, 1, 0], x: [0, 0, 1] });
connector("Axis", { at: [0, 0, L / 2], axis: [0, 0, 1] });
return spring;
