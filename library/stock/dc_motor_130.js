// 130-size brushed DC motor (Mabuchi FA-130RA and the many "TT / toy"
// motors built to its outline), 1.5–3 V.
//
// Source: Mabuchi FA-130RA datasheet — can 20.1 mm across with flats
// 15.1 mm apart, 25.1 mm long body (steel can and the plastic end cap),
// Ø6.15 mm front bearing boss 1.6 mm high, Ø2.0 mm shaft 8.0 mm beyond the
// boss, two solder tabs out of the end cap; 17 g. No tapped mounting
// holes: it is held in a clip or cradle round the can (as in Tamiya's
// gearboxes).
//
// Shaft along +Z, the can's front face on z = 0, the body down −Z.

const WIDTH = 20.1;
const FLATS = 15.1;
const BODY = 25.1;
const CAP = 6.4; // plastic end cap length
const profile = circle(WIDTH / 2).intersect(rect(WIDTH, FLATS, { center: true }));
// Steel can with its pressed front face and the two vent windows on the flats.
let can = profile.extrude(BODY - CAP).translate(0, 0, -(BODY - CAP));
for (const sy of [-1, 1]) can = can.subtract(box(5, 1.2, 3).translate(-2.5, sy > 0 ? FLATS / 2 - 0.6 : -FLATS / 2 - 0.6, -BODY + CAP + 1.5));
can = can.add(cylinder(1.6, 6.15 / 2));
// End cap with the rear bearing boss and the two brush terminals.
const cap = profile.offset(-0.15).extrude(CAP + 0.01).translate(0, 0, -BODY).add(cylinder(1.2, 2.6).translate(0, 0, -BODY - 1.2));
const tab = (x) => box(2.0, 0.3, 4.0).translate(x - 1.0, -0.15, -BODY - 3.2).subtract(cylinder(1, 0.5).rotate(90, 0, 0).translate(x, 0.5, -BODY - 2.2));
const terminals = union(tab(-5.5), tab(5.5));
const shaft = cylinder(1.6 + 8 + 4, 1.0).translate(0, 0, -4);

stock({
  partNumber: "FA-130RA",
  description: "130-size brushed DC motor (Mabuchi FA-130RA), 1.5–3 V, 20.1 × 15.1 × 25.1 mm, Ø2 mm shaft",
  manufacturer: "Mabuchi",
  mpn: "FA-130RA-2270",
  unitPrice: 1.5,
  currency: "USD",
  pack: 1,
  massG: 17,
  sources: [
    supplierSearch("Pololu", "FA-130 motor"),
    supplierSearch("Adafruit", "DC toy hobby motor 130 size"),
    supplierSearch("AliExpress", "130 dc motor 3V FA-130"),
  ],
});
metric("Shaft diameter", 2, "mm");
metric("Can width", WIDTH, "mm");
metric("Across flats", FLATS, "mm");

connector("Shaft", { at: [0, 0, 0], axis: [0, 0, 1], part: "Shaft" });
connector("Boss", { at: [0, 0, 1.6], axis: [0, 0, 1], part: "Can" });
connector("Face", { at: [0, 0, 0], axis: [0, 0, 1], part: "Can" });
connector("Rear face", { at: [0, 0, -BODY], axis: [0, 0, -1], part: "End cap" });

return [
  part("Can", can, { material: "FA-130 can, magnets and armature (lumped)" }),
  part("End cap", cap, { material: "PBT (white)" }),
  part("Terminals", terminals, { material: "Brass" }),
  part("Shaft", shaft, { material: "Steel" }),
];
