// A single-cylinder engine: crankshaft with flywheel, connecting rod, piston
// with its gudgeon pin, in a frame with two main bearings and a cylinder.
// Drawn at top dead centre; joints let the engine solve its motion exactly.
//
// Axes: the crankshaft turns about y; the piston slides along z.

const r = p.stroke / 2;           // crank throw
const L = p.rod;                  // connecting rod, centre to centre
const pinZ = r + L;               // gudgeon pin height at top dead centre
const gap = p.clearance;          // running clearance in every bearing

// A cylinder along y from y0 to y1, centred on (x, z).
const ycyl = (radius, y0, y1, x = 0, z = 0) => cylinder(y1 - y0, radius).rotate(-90, 0, 0).translate(x, y0, z);
// A box from corner to corner.
const block = (x0, y0, z0, x1, y1, z1) => box(x1 - x0, y1 - y0, z1 - z0).translate(x0, y0, z0);

// Crankshaft: main journal, two webs with counterweights, crank pin, flywheel.
const web = (y0, y1) =>
  block(-12, y0, -8, 12, y1, r + 8)
    .add(ycyl(12, y0, y1, 0, r))
    .add(ycyl(30, y0, y1).intersect(block(-30, y0, -30, 30, y1, 0)));
// The journals stop at the webs: between them the big end swings round.
const crank = ycyl(p.journal / 2, -66, -18)
  .add(ycyl(p.journal / 2, 18, 40))
  .add(web(-18, -10), web(10, 18))
  .add(ycyl(p.crankPin / 2, -10, 10, 0, r))
  .add(ycyl(56, -66, -56));

// Connecting rod: big end round the crank pin, beam, small end round the gudgeon pin.
const rod = ycyl(p.crankPin / 2 + 5, -8, 8, 0, r)
  .add(block(-5, -4, r + 6, 5, 4, pinZ - 6))
  .add(ycyl(p.gudgeon / 2 + 4, -7, 7, 0, pinZ))
  .subtract(ycyl(p.crankPin / 2 + gap, -9, 9, 0, r), ycyl(p.gudgeon / 2 + gap, -8, 8, 0, pinZ));

// Piston: crown and skirt, hollow underneath, with a boss each side of the
// small end carrying the gudgeon pin.
const pr = p.bore / 2 - gap;
const skirt = cylinder(40, pr).translate(0, 0, pinZ - 15);
const boss = (y0, y1) => ycyl(p.gudgeon / 2 + 5, y0, y1, 0, pinZ).intersect(skirt);
const piston = skirt
  .subtract(cylinder(30, pr - 4).translate(0, 0, pinZ - 16))
  .add(boss(-pr, -8), boss(8, pr))
  .subtract(ycyl(p.gudgeon / 2, -pr - 1, pr + 1, 0, pinZ));
const gudgeonPin = ycyl(p.gudgeon / 2 - 0.01, -pr + 1, pr - 1, 0, pinZ);

// Frame: base, two main bearings, two posts, and the cylinder on a flange.
const top = pinZ + 30;
const bottom = pinZ - 15 - p.stroke - 8;
const bearing = (y0, y1) => block(-22, y0, -72, 22, y1, 14).subtract(ycyl(p.journal / 2 + gap, y0 - 1, y1 + 1));
const frame = block(-70, -40, -80, 70, 40, -72)
  .add(bearing(-32, -24), bearing(24, 32))
  .add(block(-52, -8, -72, -44, 8, bottom), block(44, -8, -72, 52, 8, bottom))
  .add(block(-52, -20, bottom - 6, 52, 20, bottom).subtract(cylinder(10, p.bore / 2 + 1).translate(0, 0, bottom - 8)))
  .add(cylinder(top - bottom, p.bore / 2 + 7).translate(0, 0, bottom).subtract(cylinder(top - bottom + 2, p.bore / 2).translate(0, 0, bottom - 1)));

joint("hinge", "Crankshaft", "ground", { at: [0, 0, 0], axis: [0, 1, 0], name: "Main bearings" });
joint("hinge", "Connecting rod", "Crankshaft", { at: [0, 0, r], axis: [0, 1, 0], name: "Big end" });
joint("hinge", ["Piston", "Gudgeon pin"], "Connecting rod", { at: [0, 0, pinZ], axis: [0, 1, 0], name: "Small end" });
joint("slider", ["Piston", "Gudgeon pin"], "ground", { at: [0, 0, pinZ], axis: [0, 0, 1], name: "Cylinder bore" });
drive("Main bearings", { speed: p.rpm });

metric("Stroke", p.stroke, "mm");
metric("Swept volume", (Math.PI / 4) * p.bore ** 2 * p.stroke / 1000, "cm³");
metric("Rod ratio", L / r, "");

return [
  part("Crankshaft", crank, { material: "Steel" }),
  part("Connecting rod", rod, { material: "Steel" }),
  part("Piston", piston, { material: "Aluminium" }),
  part("Gudgeon pin", gudgeonPin, { material: "Steel" }),
  part("Frame", frame, { material: "Aluminium" }),
];
