// Shoulder yoke: a foot plate bolted to the turntable (four M3 screws into its
// inserts) and the horn-side upright with a rounded top. The shoulder servo
// rides inside the upper arm; its aluminium disc horn sits in a recess on this
// upright's inner face and is bolted to it from outside (four M3 screws, the
// horn screw's head and key through the centre), so the arm swings on the
// horn. The bearing cheek bolts onto the foot's other end.
//
// Own coordinates: the shoulder axis is the Y axis through the origin, the
// upper arm's mid-plane at y = 0, the foot's underside at z = −34.
// Prints standing on its foot.

const z0 = -ARM.yokeAxis;
const hy = hornY("MG996R", -ARM.uaHalf); // the horn's face: −32.3
const yIn = hy + HORN.MG996R.recess; // the upright's inner face: −26.3
const yOut = hy - HORN.MG996R.floor; // outer face: −36.3
const foot = 8;
let body = xy(rounded(rectP(-32, -40, 32, 46), 8), z0, z0 + foot).fillet(2, onPlane(2, z0 + foot));
const upright = rounded(circle(28).add(rectP(-28, z0 + 1, 28, 0)), 0, 0);
body = body.add(xz(upright, yOut, yIn).fillet(3, (e) => e.points.every((p) => p[2] > z0 + foot + 0.5 && (Math.abs(p[1] - yOut) < 1e-3 || Math.abs(p[1] - yIn) < 1e-3))));
// Gussets outside the upright.
for (const x0 of [-28, 24]) body = body.add(frame([x0, 0, 0], [0, 1, 0], [1, 0, 0]).place(polygon([[yOut + 0.01, z0 + foot - 0.01], [-40, z0 + foot - 0.01], [yOut + 0.01, z0 + foot + 16]]).extrude(4)));

const cuts = [];
// The horn: its x along the shoulder servo's case, which runs up the arm (+Z at rest).
cuts.push(hornSeat("MG996R", frame([0, hy, 0], [0, 0, 1], [0, -1, 0]), "Shoulder"));
ARM.yokeBolts.forEach(([x, y], i) => cuts.push(clearHole("M3", [x, y, z0 + foot], [0, 0, 1], foot, `Foot screw ${i + 1}`)));
for (const x of [-20, 20]) cuts.push(insertHole("M3", [x, 38, z0 + foot], [0, 0, 1], `Cheek insert ${x < 0 ? 1 : 2}`));
body = body.subtract(union(cuts));

conn("Seat", [0, 0, z0], [0, 0, -1], [1, 0, 0]);
conn("Cheek seat", [0, ARM.uaHalf + ARM.bearingGap, z0 + foot], [0, 0, 1], [1, 0, 0]);
return finish("Shoulder yoke", body, "PLA charcoal", [0, 0, 0], "standing on its foot");
