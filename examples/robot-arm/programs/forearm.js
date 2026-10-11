// Forearm: a hollow tapered link, two halves split lengthwise, between the
// elbow cheeks (which bolt onto its sides) and the wrist. It houses the wrist
// MG90S at its far end, tabs against the left half's 3.5 mm wall (two M2
// screws self-tapping into pilots), output out through the left face to the
// wrist housing's horn seat; a steel Ø5 dowel pin pressed into the right
// half carries the wrist housing's 625 on the other side. The servo's lead
// runs back inside. Two M3 screws (counterbored in the right half) close it.
//
// Own coordinates: the elbow axis is the Y axis, the forearm along +X, the
// wrist axis at x = 120; y as the upper arm's (faces at −20 and +16).
// `half` 0 = left, 1 = right. Each prints on its outer face.

const L = ARM.L2;
const [yL, yR] = ARM.faFaces;
const split = ARM.faSplit;
const wallL = caseAbove(SERVO.MG90S); // 3.5: the wrist servo's case top flush with the left face
const wall = 2;
const x0 = 26; // clear of the upper arm's R23 end
const side = rounded(circle(16).translate(L, 0).add(polygon([[x0, -18], [L, -16], [L, 16], [x0, 18]])), 6, 30);
const inner = side.offset(-wall);
const left = p.half < 0.5;
const lugCols = [[36, 9], [36, -9]]; // the elbow cheeks' screws
const halfCols = [[56, 9], [56, -9], [L - 30, 9], [L - 30, -9]];

let body;
const cuts = [];
if (left) {
  const yWall = yL + wallL; // −16.5
  body = xz(side, yL, split).chamfer(1.5, onPlane(1, yL)).subtract(xz(inner, yWall, split + 1));
  for (const [x, z] of [...lugCols, ...halfCols]) body = body.add(rod([x, yWall - 0.01, z], [x, split, z], 4.5));
  cuts.push(servoMount("MG90S", frame([L, yWall, 0], [-1, 0, 0], [0, -1, 0]), wallL, "M2 pilot", "Wrist servo"));
  lugCols.forEach(([x, z], i) => cuts.push(insertHole("M3", [x, yL, z], [0, -1, 0], `Cheek insert ${i + 1}`)));
  halfCols.forEach(([x, z], i) => cuts.push(insertHole("M3", [x, split, z], [0, 1, 0], `Half insert ${i + 1}`)));
  conn("Cheek seat", [0, yL, 0], [0, -1, 0], [1, 0, 0]);
  conn("Half seat", [0, split, 0], [0, 1, 0], [1, 0, 0]);
} else {
  const yWall = yR - wall; // 13.5
  body = xz(side, split, yR).chamfer(1.5, onPlane(1, yR)).subtract(xz(inner, split - 1, yWall));
  for (const [x, z] of [...lugCols, ...halfCols]) body = body.add(rod([x, split, z], [x, yWall + 0.01, z], 4.5));
  body = body.add(rod([L, 5, 0], [L, yWall + 0.01, 0], 6)); // the wrist pin's boss, clear of the servo's case (y = 1.6)
  lugCols.forEach(([x, z], i) => cuts.push(insertHole("M3", [x, yR, z], [0, 1, 0], `Cheek insert ${i + 1}`)));
  halfCols.forEach(([x, z], i) => cuts.push(clearHole("M3", [x, yR, z], [0, 1, 0], yR - split, `Screw ${i + 1}`, 3)));
  cuts.push(pinHole(5, [L, yR, 0], [0, 1, 0], 9, "Wrist pin"));
  conn("Cheek seat", [0, yR, 0], [0, 1, 0], [1, 0, 0]);
  conn("Seat", [0, split, 0], [0, -1, 0], [1, 0, 0]);
}
body = body.subtract(union(cuts));
return finish(left ? "Forearm, left half" : "Forearm, right half", body, "PLA white", left ? [90, 0, 0] : [-90, 0, 0], "lying on its outer face");
