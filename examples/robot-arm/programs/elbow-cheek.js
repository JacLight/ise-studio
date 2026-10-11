// Elbow cheeks: the forearm's yoke round the upper arm's end, bolted to the
// forearm's sides (two counterbored M3 screws each, into the forearm's
// inserts). The left cheek takes the elbow servo's disc horn in a recess on
// its inner face, bolted from outside (four M3 screws, the horn screw through
// the centre); the right cheek holds a 608 (Ø21.9 seat, 0.1 mm press, a lip
// behind it) on the upper arm's Ø8 dowel pin.
//
// Own coordinates: the elbow axis is the Y axis, the forearm along +X, y as
// the upper arm's. `side` 0 = left (horn) cheek, 1 = right (bearing) cheek.
// Each prints on its outer face.

const [yL, yR] = ARM.faFaces;
const R = 27;
const outline = rounded(circle(R).add(polygon([[0, -R], [44, -18], [50, -18], [50, 18], [44, 18], [0, R]])), 4, 20);
const lug = rounded(rectP(28, -16, 50, 16), 3);
const cols = [[36, 9], [36, -9]];
const left = p.side < 0.5;
let body;
const cuts = [];
if (left) {
  const hy = hornY("MG996R", -ARM.uaHalf); // −32.3
  const yIn = hy + HORN.MG996R.recess;
  const yOut = hy - HORN.MG996R.floor;
  body = xz(outline, yOut, yIn).fillet(3, onPlane(1, yOut)).add(xz(lug, yIn - 0.01, yL));
  // The elbow servo's case runs down the upper arm: −Z at rest.
  cuts.push(hornSeat("MG996R", frame([0, hy, 0], [0, 0, -1], [0, -1, 0]), "Elbow", { cb: 3.5 }));
  cols.forEach(([x, z], i) => cuts.push(clearHole("M3", [x, yOut, z], [0, -1, 0], yL - yOut, `Screw ${i + 1}`, 7)));
  conn("Forearm seat", [0, yL, 0], [0, 1, 0], [1, 0, 0]);
} else {
  const yIn = ARM.uaHalf + ARM.bearingGap; // 23
  const yOut = yIn + 9;
  body = xz(outline, yIn, yOut).fillet(3, onPlane(1, yOut)).add(xz(lug, yR, yIn + 0.01));
  cuts.push(bearingSeat(22, 7, [0, yIn, 0], [0, -1, 0], 2, 16, "Bearing"));
  cols.forEach(([x, z], i) => cuts.push(clearHole("M3", [x, yOut, z], [0, 1, 0], yOut - yR, `Screw ${i + 1}`, 4)));
  conn("Forearm seat", [0, yR, 0], [0, -1, 0], [1, 0, 0]);
}
body = body.subtract(union(cuts));
return finish(left ? "Elbow cheek, horn side" : "Elbow cheek, bearing side", body, "PLA charcoal", left ? [90, 0, 0] : [-90, 0, 0], "lying on its outer face");
