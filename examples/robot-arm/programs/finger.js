// Gripper fingers: two pivoting fingers geared together (involute sectors,
// module 1.5, 18 teeth, 0.1 mm backlash, centres 27 mm apart), so both close
// on the centre line. The moving one is bolted to the gripper servo's disc
// horn (four M2 screws, the horn screw through the centre); the other turns
// on a sleeve screwed into the housing. Jaw pads drop in front of the
// housing; they close to 3 mm apart at 0° and open as the servo turns.
//
// Own coordinates: the pivot on the Z axis, the finger's underside at z = 0,
// pointing +X; `side` 0 = driven finger (partner on −y), 1 = idler (partner on +y).
// Prints upside down: the top face on the bed, the jaw pad rising.

const t = 6;
const m = 1.5;
const zt = 18;
const cd = m * zt; // 27
const driven = p.side < 0.5;
const k = driven ? -1 : 1; // toward the partner
const mid = (k * cd) / 2; // the centre line between the fingers
// Gear sector toward the partner: a tooth on the line of centres for the driven
// finger, a gap for the idler.
const teethAt = driven ? -90 : 90 + 180 / zt;
const sector = polygon([[0, 0], [40 * Math.cos(rad(k * 90 - 50)), 40 * Math.sin(rad(k * 90 - 50))], [40 * Math.cos(rad(k * 90)), 40 * Math.sin(rad(k * 90))], [40 * Math.cos(rad(k * 90 + 50)), 40 * Math.sin(rad(k * 90 + 50))]]);
const teeth = gearOutline({ m, z: zt, backlash: 0.1 }).rotate(teethAt).intersect(sector);
// The finger: hub, a tapered arm swept toward the centre line, the jaw's root.
const arm = rounded(polygon([[-6, -k * 9], [12, -k * 11], [50, mid - k * 6.5], [64, mid - k * 6.5], [64, mid - k * 1.5], [50, mid - k * 1.5], [12, k * 6], [0, k * 9]]), 2, 25);
let body = xy(circle(11).add(arm), 0, t).fillet(1.2, onPlane(2, t)).add(xy(teeth, 0, t));
// Jaw pad: drops 24 mm in front of the housing, its inner face on the centre line + 1.5.
body = body.add(boxAt([50, Math.min(mid - k * 1.5, mid - k * 6.5), -24], [64, Math.max(mid - k * 1.5, mid - k * 6.5), t]).fillet(1.5, (e) => e.points.every((q) => q[2] < -23.9)));
let cuts;
if (driven) {
  // Horn's x: along the gripper servo's case, which runs toward −y in the housing.
  cuts = hornSeat("MG90S", frame([0, 0, 0], [0, -1, 0], [0, 0, 1]), "Grip", { seat: t, recess: 0 });
} else {
  cuts = cylinder(t + 2, 8.4 / 2).translate(0, 0, -1);
  conn("Pivot", [0, 0, 0], [0, 0, -1], [1, 0, 0]);
}
body = body.subtract(cuts);
return finish(driven ? "Finger, driven" : "Finger, idler", body, "PLA charcoal", [180, 0, 0], "upside down, top face on the bed");
