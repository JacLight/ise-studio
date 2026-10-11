// Base cover: closes the base and carries the base-yaw MG996R, hung under it
// in a tube by its tabs (four M2 screws from below, washers on the slotted
// tabs, into M2 heat-set inserts), output up through the cover; the 6807
// thin-section bearing is pressed into its top (Ø46.9 seat, 0.1 mm press) so
// the turntable's hub turns in the bearing, not on the servo. Four M3 screws
// in counterbores hold it to the base.
//
// Own coordinates: the base axis is Z, the cover's underside (on the base's
// rim) at z = 0. Prints upside down: the top face on the bed, the tube rising.

const s = SERVO.MG996R;
const T = 9.3; // top face: the bearing stands 0.5 mm proud of it
const mount = -(caseAbove(s) + 0.5); // the tube's end: the case top 0.5 mm under the plate
const bearingFace = mount + hornFace(s) - 7; // 6807 top flush with the horn's face
let body = cylinder(T, 62).chamfer(1.5, onPlane(2, T));
// The tube the servo's top goes up into (case along −X, toward the lead's slot).
const f = frame([0, 0, mount], [-1, 0, 0], [0, 0, 1]);
body = body.add(f.place(boxAt([-s.off - C - 9, -s.W / 2 - C - 3.5, 0], [s.L - s.off + C + 9, s.W / 2 + C + 3.5, -mount + 0.01])));
const cuts = [servoMount("MG996R", f, -mount, "M2 insert", "Servo", 6)];
cuts.push(cylinder(10, 46.9 / 2).translate(0, 0, bearingFace));
cuts.push(polygon([[0, bearingFace - 2.01], [19.2, bearingFace - 2.01], [21.2, bearingFace + 0.01], [0, bearingFace + 0.01]]).revolve(360));
cuts.push(cylinder(bearingFace + 1, 19.2).translate(0, 0, -1));
for (let k = 0; k < 4; k++) {
  const a = rad(45 + 90 * k);
  cuts.push(clearHole("M3", [55.5 * Math.cos(a), 55.5 * Math.sin(a), T], [0, 0, 1], T, `Screw ${k + 1}`, 3.5));
}
body = body.subtract(union(cuts));

conn("Seat", [0, 0, 0], [0, 0, -1], [1, 0, 0]);
conn("Bearing", [0, 0, bearingFace], [0, 0, 1]);
return finish("Base cover", body, "PLA charcoal", [180, 0, 0], "upside down, top face on the bed");
