// Wrist housing: the gripper's body. Its two cheeks straddle the forearm's
// end: the left one takes the wrist servo's disc horn in a recess, bolted
// from outside (four M2 screws, the horn screw through the centre), the right
// one holds a 625 (Ø15.9 seat, 0.1 mm press) on the forearm's Ø5 dowel pin.
// Inside, the gripper MG90S hangs under the top wall by its tabs (two M2
// screws self-tapping into pilots), output up through it to the moving
// finger; its lead leaves through a grommet hole in the side. The other
// finger turns on a sleeve screwed into an M3 insert in the top. A bottom
// cover closes it (four M3 screws into corner inserts).
//
// Own coordinates: the wrist axis is the Y axis, the gripper pointing +X,
// y as the forearm's. Prints upside down (top face on the bed); the cheeks'
// horn recess and bearing seat print as horizontal holes (Ø17 / Ø15.9: a
// steep arch at the top, no support).

const s = SERVO.MG90S;
const [yL, yR] = ARM.faFaces;
const hy = hornY("MG90S", yL); // −29.7
const yLin = hy + HORN.MG90S.recess; // −24.7
const yLout = hy - HORN.MG90S.floor; // −32.7
const yRin = yR + ARM.bearingGap; // 17
const yRout = yRin + 7; // 625: 5 wide + 2 lip
const zTop = 16;
const zBot = -13.5;
const xF = 22; // the body's back face, clear of the forearm's R16 end
const xN = 64;
const plan = rounded(rectP(xF, yLout, xN, yRout), 7);
let body = xy(plan, zBot, zTop).fillet(3, onPlane(2, zTop));
body = body.subtract(xy(plan.offset(-2), zBot - 1, zTop - caseAbove(s)));
// Cheeks back to the wrist axis.
const cheek = rounded(circle(18).add(rectP(0, -16, xF + 6, zTop)), 0, 0).intersect(rectP(-20, zBot, xF + 6, zTop));
// Each cheek: a 3 mm plate, full thickness only round the horn / bearing (R11) and at its root.
const root = rectP(16, zBot, xF + 6, zTop);
body = body.add(xz(cheek, yLout, yLout + 3), xz(cheek, yRout - 3, yRout), xz(root, yLout, yLin), xz(root, yRin, yRout));
body = body.add(rod([0, yLout + 0.01, 0], [0, yLin, 0], 11), rod([0, yRin, 0], [0, yRout - 0.01, 0], 11));
// Corner columns for the cover's inserts, and a boss under the finger pivot's insert.
const corners = [[xF + 5, yLout + 5], [xN - 5, yLout + 5], [xF + 5, yRout - 5], [xN - 5, yRout - 5]];
for (const [x, y] of corners) body = body.add(cylinder(zTop - zBot - 1, 4.5).translate(x, y, zBot));
const gx = 40;
const gy = 5; // the gripper servo's axis (the moving finger's pivot)
const pivot = [gx, gy - 27, zTop]; // the other finger's pivot: gear centres 27 apart (m 1.5, 18 teeth)
body = body.add(cylinder(1.01, 4).translate(pivot[0], pivot[1], zTop - caseAbove(s) - 1));

const cuts = [];
cuts.push(hornSeat("MG90S", frame([0, hy, 0], [-1, 0, 0], [0, -1, 0]), "Wrist"));
cuts.push(bearingSeat(16, 5, [0, yRin, 0], [0, -1, 0], 2, 11, "Wrist bearing"));
cuts.push(servoMount("MG90S", frame([gx, gy, zTop - caseAbove(s)], [0, -1, 0], [0, 0, 1]), caseAbove(s), "M2 pilot", "Grip servo"));
cuts.push(insertHole("M3 short", pivot, [0, 0, 1], "Pivot insert"));
corners.forEach(([x, y], i) => cuts.push(insertHole("M3", [x, y, zBot], [0, 0, -1], `Cover insert ${i + 1}`)));
cuts.push(rod([gx, yLout - 1, -3.1], [gx, yLout + 3.5, -3.1], 3.5)); // the gripper servo's lead out
body = body.subtract(union(cuts));

conn("Finger pivot", pivot, [0, 0, 1], [1, 0, 0]);
conn("Cover seat", [43, 0, zBot], [0, 0, -1], [1, 0, 0]);
return finish("Wrist housing", body, "PLA white", [180, 0, 0], "upside down, top face on the bed");
