// Small printed parts, chosen by `which`:
//   0  Wrist cover: closes the wrist housing's underside (four M3 screws into
//      its corner inserts). Own coordinates: the wrist axis is Y, the gripper
//      +X, the housing's open face at z = −13.5. Prints flat.
//   1  Finger pivot spacer: holds the idler finger 0.3 mm above... a Ø11 tube
//      under the idler finger, on the housing's top. Prints standing.
//   2  Finger pivot sleeve: the idler finger turns on its Ø8 shaft (0.2 mm a
//      side), its flange keeps the finger on, an M3 screw clamps it into the
//      housing's insert. Prints flange down.

const w = Math.round(p.which);
if (w === 0) {
  const [yL, yR] = ARM.faFaces;
  const yLout = yL - (hornGap("MG90S") + HORN.MG90S.recess) - HORN.MG90S.floor;
  const yRout = yR + ARM.bearingGap + 7;
  const zBot = -13.5;
  const xF = 22;
  const xN = 64;
  let body = xy(rounded(rectP(xF, yLout, xN, yRout), 7), zBot - 2.5, zBot).fillet(1.5, onPlane(2, zBot - 2.5));
  const corners = [[xF + 5, yLout + 5], [xN - 5, yLout + 5], [xF + 5, yRout - 5], [xN - 5, yRout - 5]];
  body = body.subtract(union(corners.map(([x, y], i) => clearHole("M3", [x, y, zBot - 2.5], [0, 0, -1], 2.5, `Screw ${i + 1}`))));
  conn("Seat", [43, 0, zBot], [0, 0, 1], [1, 0, 0]);
  return finish("Wrist cover", body, "PLA charcoal", [0, 0, 0], "flat");
}
if (w === 1) {
  const body = cylinder(9.4, 5.5).subtract(cylinder(9.6, 8.2 / 2).translate(0, 0, -0.1)).chamfer(0.5, onPlane(2, 9.4));
  conn("Bottom", [0, 0, 0], [0, 0, -1], [1, 0, 0]);
  return finish("Finger pivot spacer", body, "PLA charcoal", [0, 0, 0], "standing");
}
// The sleeve: shaft from the housing's top (z = 0) up through the spacer and the
// finger to the flange 0.3 mm over the finger (finger 9.7…15.7 above the top).
const shaft = 16;
let body = cylinder(shaft, 4).add(cylinder(1.5, 6).translate(0, 0, shaft)).chamfer(0.5, onPlane(2, shaft + 1.5));
body = body.subtract(cylinder(shaft + 3, CLEAR.M3 / 2).translate(0, 0, -1));
conn("Bottom", [0, 0, 0], [0, 0, -1], [1, 0, 0]);
conn("Screw", [0, 0, shaft + 1.5], [0, 0, 1]);
return finish("Finger pivot sleeve", body, "PLA charcoal", [180, 0, 0], "flange down");
