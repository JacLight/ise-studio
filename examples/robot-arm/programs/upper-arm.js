// Upper arm: a hollow dog-bone link, two halves split lengthwise. It houses
// both big servos: the shoulder MG996R at its lower end and the elbow MG996R
// at its upper end, each held by its tabs against the left half's 6.9 mm wall
// (four M2 screws, washers on the slotted tabs, into M2 heat-set inserts), its
// case top flush with the left face and its output spline out through it.
// The shoulder servo's horn is bolted into the yoke, so the arm swings on it;
// the elbow servo's horn is bolted into the forearm. On the right half, steel
// Ø8 dowel pins pressed into bosses run in the 608s of the shoulder cheek and
// the forearm's elbow cheek. Six M3 screws (counterbored in the right half)
// close it into the left half's inserts.
//
// Own coordinates: the shoulder axis is the Y axis, the arm along +Z, the
// elbow axis at z = 130; mid-plane y = 0, faces at y = ±22.
// `half` 0 = left (servo) half, 1 = right half. Each prints on its outer face.

const s = SERVO.MG996R;
const L = ARM.L1;
const [R0, R1] = ARM.uaR;
const H = ARM.uaHalf;
const split = ARM.uaSplit;
const wall = 2.5;
const bone = rounded(circle(R0).add(circle(R1).translate(0, L), polygon([[-15, 18], [15, 18], [13, L - 18], [-13, L - 18]])), 0, 45);
const inner = bone.offset(-wall);
const cols = [[15.5, 0], [-15.5, 0], [8, 65], [-8, 65], [14.5, L], [-14.5, L]];
const left = p.half < 0.5;

let body;
const cuts = [];
if (left) {
  const yWall = -H + ARM.uaWall; // −15.1: the servos' mounting face
  body = xz(bone, -H, split).chamfer(1.5, onPlane(1, -H)).subtract(xz(inner, yWall, split + 1));
  for (const [x, z] of cols) body = body.add(rod([x, yWall - 0.01, z], [x, split, z], 4.5));
  const shoulder = frame([0, yWall, 0], [0, 0, 1], [0, -1, 0]); // case up the arm, output −y
  const elbow = frame([0, yWall, L], [0, 0, -1], [0, -1, 0]); // case down the arm
  cuts.push(servoMount("MG996R", shoulder, ARM.uaWall, "M2 insert", "Shoulder servo", 5));
  cuts.push(servoMount("MG996R", elbow, ARM.uaWall, "M2 insert", "Elbow servo", 5));
  cols.forEach(([x, z], i) => cuts.push(insertHole("M3", [x, split, z], [0, 1, 0], `Half insert ${i + 1}`)));
  conn("Half seat", [0, split, 0], [0, 1, 0], [1, 0, 0]);
} else {
  const yWall = H - wall; // 19.5
  body = xz(bone, split, H).chamfer(1.5, onPlane(1, H)).subtract(xz(inner, split - 1, yWall));
  for (const [x, z] of cols) body = body.add(rod([x, split, z], [x, yWall + 0.01, z], 4.5));
  // Bosses for the dowel pins, clear of the servos' case bottoms (y = 14).
  for (const z of [0, L]) body = body.add(rod([0, 14.5, z], [0, yWall + 0.01, z], 8));
  cols.forEach(([x, z], i) => cuts.push(clearHole("M3", [x, H, z], [0, 1, 0], H - split, `Screw ${i + 1}`, 3)));
  cuts.push(pinHole(8, [0, H, 0], [0, 1, 0], H - 14.5, "Shoulder pin"));
  cuts.push(pinHole(8, [0, H, L], [0, 1, 0], H - 14.5, "Elbow pin"));
  conn("Seat", [0, split, 0], [0, -1, 0], [1, 0, 0]);
}
body = body.subtract(union(cuts));
return finish(left ? "Upper arm, left half" : "Upper arm, right half", body, "PLA white", left ? [90, 0, 0] : [-90, 0, 0], "lying on its outer face");
