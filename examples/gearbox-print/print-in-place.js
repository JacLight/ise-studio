// The same two-stage reducer, redesigned to print in one piece on an FDM
// printer and turn straight off the bed. Nothing is assembled, and nothing
// needs slicer supports:
//
// - Three rotating bodies (input shaft + pinion; intermediate wheel + pinion;
//   output wheel + shaft) sit in a housing printed around them. The housing
//   is a solid block with each body's swept volume cut out, grown by a
//   running clearance sideways and one layer gap above and below.
// - The housing never overhangs: every pocket is open to the top. Gears
//   rest on the floor (or the stage-1 wheel) across that layer gap.
// - Each shaft ends in a 45° cone in the housing floor: it cannot lift out,
//   and the cone and its seat print without supports.
// - Stage-2 gears are open to the top, so the train can be watched turning;
//   a knob drives the input and a pointer shows the output.
// - Gear faces print over that layer gap, which works like a support
//   interface (the slicer's own supports leave the same 0.2–0.4 mm). Bambu
//   Studio calls these faces "floating cantilevers"; ties that would satisfy
//   it (one every ~7 mm) could never be snapped by turning, so there are none.

const s1 = { m: p.module1, z1: p.pinion1, z2: p.gear1 };
const s2 = { m: p.module2, z1: p.pinion2, z2: p.gear2 };
const tip = (m, z) => pitchRadius(m, z) + m;
const c = p.clearance; // radial running clearance (gear tips, shafts)
const g = p.layerGap; // axial gap between faces printed one over the other
const rs = p.shaft / 2;
const floor = p.floor;

// Centre distances opened by the mesh gap so printed teeth never fuse.
const a1 = pitchRadius(s1.m, s1.z1) + pitchRadius(s1.m, s1.z2) + p.meshGap;
const a2 = pitchRadius(s2.m, s2.z1) + pitchRadius(s2.m, s2.z2) + p.meshGap;
const xIn = 0, xMid = a1, xOut = a1 + a2;

// Heights, from the floor's top (z = 0). The housing's top is flush with stage 2.
const z1 = g, z1top = z1 + p.width1;
const z2 = z1top + g, z2top = z2 + p.width2;
const top = z2top;
const head = 1.6; // how far each shaft's cone flares out

const plain = (m, z, width) => gearOutline({ m, z, backlash: p.backlash }).extrude(width);

/** A shaft from the bed up to `to`, with its locking cone at the bottom. */
const shaftTo = (x, to) =>
  cylinder(head, rs + head, { top: rs })
    .add(cylinder(to + floor - head + 0.01, rs).translate(0, 0, head - 0.01))
    .translate(x, 0, -floor);

// ---- The rotating bodies -------------------------------------------------------

// Input: shaft and stage-1 pinion, then a column up to a knurled knob above
// the housing. The column clears the stage-1 wheel; the knob flares out of it
// at 45°, so nothing on this body overhangs.
const columnR = Math.min(pitchRadius(s1.m, s1.z1) - 1.25 * s1.m, a1 - tip(s1.m, s1.z2) - c - 0.2);
const knobR = Math.min(16, a1 - tip(s2.m, s2.z1) - c - 2);
const knobH = 6;
const flare = knobR - columnR;
let knob = cylinder(flare, columnR, { top: knobR }).add(cylinder(knobH, knobR).translate(0, 0, flare - 0.01));
for (let i = 0; i < 18; i++) {
  const a = (i / 18) * 2 * Math.PI;
  knob = knob.subtract(cylinder(flare + knobH + 2, 1.2).translate(knobR * Math.cos(a), knobR * Math.sin(a), -1)); // grooves the full height: no ceilings over air
}
knob = knob.add(cylinder(10, 2.5).translate(knobR - 5, 0, flare + knobH - 0.02)); // crank handle
const input = shaftTo(xIn, top)
  .add(plain(s1.m, s1.z1, p.width1).translate(xIn, 0, z1))
  .add(cylinder(top + g - z1top + 0.02, columnR).translate(xIn, 0, z1top - 0.01))
  .add(knob.translate(xIn, 0, top + g));

// Intermediate: stage-1 wheel with the stage-2 pinion standing on it.
const middle = shaftTo(xMid, top)
  .add(plain(s1.m, s1.z2, p.width1).rotate(0, 0, 180 + 180 / s1.z2).translate(xMid, 0, z1))
  .add(plain(s2.m, s2.z1, top - z1top + 0.01).translate(xMid, 0, z1top - 0.01));

// Output: stage-2 wheel on its shaft, with a pointer to show it turning.
const wheelR = tip(s2.m, s2.z2);
const output = shaftTo(xOut, top)
  .add(plain(s2.m, s2.z2, p.width2).rotate(0, 0, 180 + 180 / s2.z2).translate(xOut, 0, z2))
  .add(cylinder(4, rs + 3).translate(xOut, 0, top - 0.01))
  .add(box(wheelR * 0.62, 4, 2).translate(xOut + rs, -2, top - 0.01))
  .add(cylinder(2, 4, { top: 0.01 }).rotate(0, 90, 0).translate(xOut + rs + wheelR * 0.62 - 0.01, 0, top + 1));

// ---- The housing: a block with each body's swept volume cut out -------------------

const sweep = (x, r, from, to) => cylinder(to - from, r).translate(x, 0, from);
const cone = (x) => cylinder(head + c * 0.42, rs + head + c * 1.42, { top: rs + c }).translate(x, 0, -floor - 0.01);
// Every pocket is open to the top: the housing has no roof to print over a
// gear (the shafts' cones hold the gears in), so it never overhangs at all.
const cavities = [
  sweep(xIn, tip(s1.m, s1.z1) + c, z1 - g, top + 1),
  sweep(xMid, tip(s1.m, s1.z2) + c, z1 - g, top + 1),
  sweep(xOut, wheelR + c, z2 - g, top + 1),
  ...[xIn, xMid, xOut].flatMap((x) => [sweep(x, rs + c, -floor - 1, top + 1), cone(x)]),
];

const margin = Math.max(tip(s1.m, s1.z2), wheelR) + c + p.wall;
const left = xIn - Math.max(tip(s1.m, s1.z1) + c + p.wall, knobR + 2);
const right = xOut + wheelR + c + p.wall;
const outline = polygon([[left, -margin], [right, -margin], [right, margin], [left, margin]]).offset(-8).offset(8);
// Corner holes for screwing it down.
const corners = [[left + 7, -margin + 7], [right - 7, -margin + 7], [right - 7, margin - 7], [left + 7, margin - 7]];
const housing = outline
  .subtract(corners.map(([x, y]) => circle(2.2).translate(x, y)))
  .extrude(top + floor)
  .translate(0, 0, -floor)
  .subtract(cavities);

// ---- Check the print will turn ------------------------------------------------------

const bodies = [["Input shaft", input], ["Intermediate gears", middle], ["Output wheel", output], ["Housing", housing]];
let smallest = Infinity;
for (let i = 0; i < bodies.length; i++)
  for (let j = i + 1; j < bodies.length; j++) {
    const d = bodies[i][1].minGap(bodies[j][1], 2);
    if (d < 0.2) throw new Error(`${bodies[i][0]} and ${bodies[j][0]} are only ${d.toFixed(2)} mm apart: they would print fused`);
    smallest = Math.min(smallest, d);
  }
metric("Smallest gap between moving parts", Math.round(smallest * 100) / 100, "mm");
metric("Reduction", Math.round(((s1.z2 / s1.z1) * (s2.z2 / s2.z1)) * 100) / 100, ": 1");

const print = { material: "PLA", process: "3d_print" };
return [
  part("Housing", housing, print),
  part("Input shaft", input, print),
  part("Intermediate gears", middle, print),
  part("Output wheel", output, print),
];
