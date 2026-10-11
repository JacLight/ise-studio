// Turntable: a Ø118 disc (a step in from the Ø124 base) bolted to the base
// servo's aluminium disc horn — four M3 screws in counterbores into the horn's
// tapped holes, the horn screw through the centre — with a hub pressed into
// the 6807's bore so the bearing carries the arm's tipping moment. Four M3
// heat-set inserts take the shoulder yoke.
//
// Own coordinates: the base axis is Z, the horn's face (the hub's shoulder)
// at z = 0. Prints upside down: the top face on the bed, the hub on top.

const top = 10;
let body = cylinder(top - 0.7, 59).translate(0, 0, 0.7).chamfer(1.5, onPlane(2, top)).fillet(2, onPlane(2, 0.7));
body = body.add(cylinder(0.71, 20)); // boss on the bearing's inner ring and the horn
body = body.add(cylinder(7, 35.02 / 2).subtract(cylinder(7.2, 12.4).translate(0, 0, -0.1)).translate(0, 0, -7)); // hub in the 6807's Ø35 bore
const cuts = [];
// The base servo's case runs along −X, so the horn's x does too.
cuts.push(hornSeat("MG996R", frame([0, 0, 0], [-1, 0, 0], [0, 0, 1]), "Base", { seat: top, recess: 0, cb: 3.5 }));
ARM.yokeBolts.forEach(([x, y], i) => cuts.push(insertHole("M3", [ARM.yokeX + x, y, top], [0, 0, 1], `Yoke insert ${i + 1}`)));
body = body.subtract(union(cuts));

conn("Yoke seat", [ARM.yokeX, 0, top], [0, 0, 1], [1, 0, 0]);
return finish("Turntable", body, "PLA white", [180, 0, 0], "upside down, top face on the bed");
