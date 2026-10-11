// Shoulder bearing cheek: holds a 608 (Ø21.9 seat, 0.1 mm press, a lip
// behind it) on the steel dowel pin in the upper arm's side, so the shoulder
// is carried on both sides: the servo's horn in the yoke, this bearing here.
// It goes on last, over the pin, and bolts to the yoke's foot (two M3 screws
// into inserts).
//
// Own coordinates: the shoulder axis is the Y axis, the upper arm's mid-plane
// y = 0, the yoke foot's top at z = −26. Prints lying on its inner face.

const yIn = ARM.uaHalf + ARM.bearingGap; // 23
const yOut = yIn + 9;
const zFoot = -ARM.yokeAxis + 8;
let body = xz(circle(22).add(rectP(-22, zFoot, 22, 0)), yIn, yOut).fillet(2.5, (e) => e.points.every((p) => p[2] > zFoot + 0.5 && Math.abs(p[1] - yOut) < 1e-3));
body = body.add(boxAt([-26, yIn, zFoot], [26, 46, zFoot + 6]).fillet(2, (e) => e.points.every((p) => p[2] > zFoot + 5.9 && p[1] > yOut)));
for (const x0 of [-26, 23.5]) body = body.add(frame([x0, 0, 0], [0, 1, 0], [1, 0, 0]).place(polygon([[yOut - 0.01, zFoot + 5.99], [44, zFoot + 5.99], [yOut - 0.01, zFoot + 20]]).extrude(2.5)));
const cuts = [bearingSeat(22, 7, [0, yIn, 0], [0, -1, 0], 2, 16, "Bearing")];
for (const x of [-20, 20]) cuts.push(clearHole("M3", [x, 38, zFoot + 6], [0, 0, 1], 6, `Foot screw ${x < 0 ? 1 : 2}`));
body = body.subtract(union(cuts));

conn("Seat", [0, yIn, zFoot], [0, 0, -1], [1, 0, 0]);
return finish("Shoulder bearing cheek", body, "PLA charcoal", [90, 0, 0], "lying on its inner face");
