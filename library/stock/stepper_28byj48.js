// 28BYJ-48 geared unipolar stepper motor (5 V and 12 V): a Ø28 mm can-stack
// stepper with a 1:64 (63.68:1) spur gearbox, 2048 steps per output turn
// in full-step mode, the output shaft off-centre.
//
// Source: Kiatronics / Kiatronics-style 28BYJ-48 datasheet drawing — can
// Ø28 mm × 19 mm, two R3.5 mounting ears with Ø4.2 mm holes 35 mm apart
// (42 mm across the ears), 0.8 mm thick, flush with the top face; output shaft Ø5 mm with two flats
// 3 mm across, 6 mm long, standing 10 mm above the face on a Ø9 × 1.5 mm
// boss, its axis 8 mm off the can's; the blue wire cover on the can's
// side 14.6 mm wide; 5-wire lead to a 5-pin JST-XH; about 30 g.
//
// The top face (ears) on z = 0, the can down −Z, the output shaft up +Z at
// (0, 8); ears along X, wire cover towards −Y.

const VOLTS = { 5: [1.6, "5V"], 12: [2.1, "12V"] };
const volts = p.volts;
const [price, tag] = lookup(VOLTS, volts, "28BYJ-48 voltage");

const R = 14;
const H = 19;
// Can: the steel can, and the top plate with the two ears.
const can = cylinder(H - 0.8, R).translate(0, 0, -H);
let top = cylinder(0.8, R).translate(0, 0, -0.8);
const ear = cylinder(0.8, 3.5).add(box(7, 7, 0.8).translate(-3.5, -3.5, 0)); // rounded ear, squared towards the can
for (const sx of [-1, 1]) {
  top = top.add(ear.translate(sx * 17.5, 0, -0.8).add(box(4, 7, 0.8).translate(sx > 0 ? 13 : -17, -3.5, -0.8)));
}
top = top.subtract(...[-1, 1].map((sx) => cylinder(1.2, 2.1).translate(sx * 17.5, 0, -1)));
// Rounded ear ends: trim the squared corners back to the R3.5 round.
const trim = (sx) => box(3.6, 8, 2).translate(sx > 0 ? 17.5 : -21.1, -4, -1.5).subtract(cylinder(3, 3.5).translate(sx * 17.5, 0, -2));
top = top.subtract(trim(-1), trim(1));
// Gearbox cover bump round the output, the boss and the shaft.
const boss = cylinder(1.5, 4.5).translate(0, 8, 0);
const flatCut = (s) => box(6, 3, 6.01).translate(-3, s > 0 ? 1.5 : -4.5, 4).translate(0, 8, 0);
const shaft = cylinder(10 + 2, 2.5).translate(0, 8, -2).subtract(flatCut(1), flatCut(-1));
// Wire cover on the −Y side: a box from the can's wall outwards.
const cover = box(14.6, 5, 16.5).translate(-7.3, -R - 3.2, -H + 0.5).subtract(cylinder(H + 1, R - 0.01).translate(0, 0, -H - 0.5));
const wires = union(...[-2.4, -1.2, 0, 1.2, 2.4].map((x) => cylinder(30, 0.5).rotate(90, 0, 0).translate(x, -R - 3.2 + 0.01, -H + 3)));

stock({
  partNumber: `28BYJ-48-${tag}`,
  description: `28BYJ-48 geared stepper motor, ${volts} V unipolar, 1:64 gearbox (2048 steps/rev), Ø28 mm, Ø5 mm double-flat shaft, 5-pin JST-XH lead`,
  mpn: `28BYJ-48 ${tag}`,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG: 30,
  sources: [
    supplierSearch("Adafruit", "28BYJ-48 stepper motor"),
    supplierSearch("AliExpress", `28BYJ-48 ${tag} stepper motor`),
    supplierSearch("Amazon", `28BYJ-48 ${tag} stepper motor ULN2003`),
  ],
});
metric("Steps per output turn", 2048);
metric("Gear ratio", 63.68);
metric("Hole spacing", 35, "mm");
metric("Shaft diameter", 5, "mm");
metric("Shaft offset", 8, "mm");

connector("Shaft", { at: [0, 8, 0], axis: [0, 0, 1], x: [0, 1, 0], part: "Shaft" });
connector("Face", { at: [0, 0, 0], axis: [0, 0, 1], part: "Can" });
connector("Hole 1", { at: [-17.5, 0, 0], axis: [0, 0, 1], part: "Can" });
connector("Hole 2", { at: [17.5, 0, 0], axis: [0, 0, 1], part: "Can" });
connector("Rear face", { at: [0, 0, -H], axis: [0, 0, -1], part: "Can" });
connector("Cable exit", { at: [0, -R - 3.2, -H + 3], axis: [0, -1, 0], part: "Wire cover" });

return [
  part("Can", union(can, top, boss), { material: "28BYJ-48 can, gears and windings (lumped)" }),
  part("Shaft", shaft, { material: "Steel" }),
  part("Wire cover", cover, { material: "ABS (blue)" }),
  part("Leads", wires, { material: "Hook-up wire (PVC, copper)" }),
];
