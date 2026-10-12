// N20 micro metal gear motor (the "micro metal gearmotor" form): a 12 mm
// brushed DC motor with flats behind a 12 × 10 mm spur gearbox, Ø3 mm
// D-shaft, 6 V.
//
// Source: Pololu micro metal gearmotor dimension drawing (items #2361 …
// #2386, HP 6 V): gearbox 12 × 10 mm, 9 mm long; motor can Ø12 mm with
// flats 10 mm apart, 15 mm long including the end cap; Ø3 mm D-shaft
// (2.5 mm across the flat) 10 mm from the gearbox face, through a Ø4 mm
// bushing 0.8 mm proud; two M1.6 tapped holes 9 mm apart in the gearbox
// face; 9.5 g. No-load speeds from Pololu's HP 6 V table. The generic N20
// motors sold on AliExpress keep these dimensions.
//
// Shaft along +Z, the gearbox face on z = 0, motor behind it down −Z.

const GEARS = {
  // ratio: [no-load rpm at 6 V, stall torque kg·cm]
  30: [1000, 0.6],
  50: [625, 1.0],
  100: [320, 2.0],
  150: [210, 2.8],
  298: [100, 5.4],
};
const ratio = p.ratio;
const [rpm, stall] = lookup(GEARS, ratio, "N20 gear ratio");

const W = 12;
const T = 10;
const GB = 9;
const MOTOR = 15;
// Gearbox: front and rear plates with the gear stack between them, a hair inset.
const plate = box(W, T, 1).translate(-W / 2, -T / 2, 0);
let gearbox = union(
  plate.translate(0, 0, -1),
  box(W - 0.6, T - 0.6, GB - 2 + 0.02).translate(-W / 2 + 0.3, -T / 2 + 0.3, -GB + 0.99),
  plate.translate(0, 0, -GB),
  cylinder(0.8, 2).translate(0, 0, -0.01),
);
for (const x of [-4.5, 4.5]) gearbox = gearbox.subtract(internalThreadTool({ d: 1.6, pitch: 0.35, z0: -2.5, z1: 0 }).translate(x, 0, 0));
// Motor can (flats top and bottom) with the black end cap and its two terminals.
const canProfile = circle(W / 2).intersect(rect(W, T, { center: true }));
const can = canProfile.extrude(MOTOR - 2).translate(0, 0, -GB - MOTOR + 2);
const cap = canProfile.offset(-0.3).extrude(2.01).translate(0, 0, -GB - MOTOR);
const tab = (x) => box(1.6, 0.3, 2.6).translate(x - 0.8, -0.15, -GB - MOTOR - 2.5).subtract(cylinder(1, 0.4).rotate(90, 0, 0).translate(x, 0.5, -GB - MOTOR - 1.6));
const terminals = union(tab(-3.5), tab(3.5));
// Shaft: Ø3 with the D-flat (2.5 across) over its outer 8 mm.
const shaft = cylinder(10 + 3, 1.5).translate(0, 0, -3).subtract(box(4, 2, 8.01).translate(-2, 1.0, 2));

stock({
  partNumber: `N20-6V-${ratio}:1`,
  description: `N20 micro metal gear motor, 6 V, ${ratio}:1 (${rpm} rpm no load), 12 × 10 × 24 mm, Ø3 mm D-shaft`,
  unitPrice: cents(3.2 + ratio / 300),
  currency: "USD",
  pack: 1,
  massG: 9.5,
  sources: [
    supplierSearch("Pololu", `${ratio}:1 micro metal gearmotor HP 6V`),
    supplierSearch("AliExpress", `N20 gear motor 6V ${rpm}rpm`),
    supplierSearch("Amazon", `N20 micro gear motor 6V ${rpm} rpm`),
  ],
});
metric("Gear ratio", ratio);
metric("No-load speed", rpm, "rpm");
metric("Stall torque", stall, "kg·cm");
metric("Shaft diameter", 3, "mm");

connector("Shaft", { at: [0, 0, 0], axis: [0, 0, 1], x: [0, 1, 0], part: "Shaft" });
connector("Face", { at: [0, 0, 0], axis: [0, 0, 1], part: "Gearbox" });
connector("Hole 1", { at: [-4.5, 0, 0], axis: [0, 0, 1], part: "Gearbox" });
connector("Hole 2", { at: [4.5, 0, 0], axis: [0, 0, 1], part: "Gearbox" });
connector("Rear face", { at: [0, 0, -GB - MOTOR], axis: [0, 0, -1], part: "Motor" });

return [
  part("Gearbox", gearbox, { material: "N20 gearbox (steel plates, brass gears — lumped)" }),
  part("Motor", can, { material: "N20 motor (can, magnets, armature — lumped)" }),
  part("End cap", cap, { material: "PBT (black)" }),
  part("Terminals", terminals, { material: "Brass" }),
  part("Shaft", shaft, { material: "Steel" }),
];
