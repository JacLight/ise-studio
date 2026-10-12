// Base: a stepped round housing — a Ø150 foot flange under a Ø124 drum — open
// at the top for the base cover (which carries the base-yaw servo and the
// turntable bearing). Four bosses with M3 heat-set inserts take the cover's
// screws; four counterbored Ø4.5 holes in the flange screw it to the bench;
// the servo's lead leaves through a slot at the back.
//
// Own coordinates: the base's axis is Z, the bench at z = 0.
// Prints upright, the floor on the bed.

const rim = 58;
const R = 62;
let body = cylinder(6, 75).fillet(3, onPlane(2, 6)).add(cylinder(rim, R).chamfer(1, onPlane(2, rim)));
body = body.subtract(cylinder(rim, R - 3).translate(0, 0, 3));
for (let k = 0; k < 4; k++) {
  const a = rad(45 + 90 * k);
  body = body.add(cylinder(rim - 3, 5.5).translate(55.5 * Math.cos(a), 55.5 * Math.sin(a), 3));
}
const cuts = [];
for (let k = 0; k < 4; k++) {
  const a = rad(45 + 90 * k);
  cuts.push(insertHole("M3", [55.5 * Math.cos(a), 55.5 * Math.sin(a), rim], [0, 0, 1], `Insert ${k + 1}`));
  const b = rad(90 * k);
  const [x, y] = [69 * Math.cos(b), 69 * Math.sin(b)];
  cuts.push(cylinder(8, 2.25).translate(x, y, -1), cylinder(3, 4.25).translate(x, y, 4));
}
// The lead's slot at the back (−X), clear of the floor.
cuts.push(boxAt([-R - 1, -7, 8], [-R + 6, 7, 32]));
body = body.subtract(union(cuts));

conn("Ground", [0, 0, 0], [0, 0, -1], [1, 0, 0]);
conn("Cover seat", [0, 0, rim], [0, 0, 1], [1, 0, 0]);
return finish("Base", body, "PLA charcoal", [0, 0, 0], "upright, floor on the bed");
