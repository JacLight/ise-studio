// Two-stage spur reducer between two plates. Every gear, shaft, bearing,
// spacer and plate is its own part, so each can be made and checked.
const s1 = { m: p.module1, z1: p.pinion1, z2: p.gear1 };
const s2 = { m: p.module2, z1: p.pinion2, z2: p.gear2 };
// Shaft centres: input at the origin, intermediate along +X, output beyond it.
const a1 = pitchRadius(s1.m, s1.z1) + pitchRadius(s1.m, s1.z2);
const a2 = pitchRadius(s2.m, s2.z1) + pitchRadius(s2.m, s2.z2);
const xIn = 0, xMid = a1, xOut = a1 + a2;

const fit = 0.02; // running clearance between bores and shafts
const shaftD = p.shaft;
const bearing = { d: shaftD, D: shaftD * 2.2, B: 7 };
const plateT = p.plate;
const gap = 1; // axial gap between neighbouring parts
// Axial stack, from the front plate's inner face (z = 0).
const zGear1 = bearing.B - plateT + gap + 1;
const zGear2 = zGear1 + p.width1 + gap + 6;
const inner = zGear2 + p.width2 + gap + 2; // distance between the plates
const zBack = inner;

const parts = [];
const steel = { material: "Steel" };

// Stage 1: pinion on the input shaft, wheel on the intermediate shaft.
parts.push(part("Input pinion", gear({ m: s1.m, z: s1.z1, width: p.width1, bore: shaftD + fit, backlash: p.backlash }).translate(xIn, 0, zGear1), { ...steel, process: "cnc_mill" }));
parts.push(
  part(
    "Stage 1 wheel",
    gear({ m: s1.m, z: s1.z2, width: p.width1, bore: shaftD + fit, backlash: p.backlash })
      .rotate(0, 0, 180 + 180 / s1.z2) // a tooth gap faces the pinion
      .translate(xMid, 0, zGear1),
    { material: "Acetal", process: "cnc_mill" },
  ),
);
// Stage 2: pinion on the intermediate shaft, output wheel.
parts.push(
  part("Intermediate pinion", gear({ m: s2.m, z: s2.z1, width: p.width2, bore: shaftD + fit, backlash: p.backlash }).translate(xMid, 0, zGear2), { ...steel, process: "cnc_mill" }),
);
parts.push(
  part(
    "Output wheel",
    gear({ m: s2.m, z: s2.z2, width: p.width2, bore: shaftD + fit, backlash: p.backlash })
      .rotate(0, 0, 180 + 180 / s2.z2)
      .translate(xOut, 0, zGear2),
    { material: "Acetal", process: "cnc_mill" },
  ),
);

// Shafts, turned: they run through both plates' bearings; input and output stick out.
const shaft = (name, x, front, back) =>
  part(name, cylinder(inner + 2 * plateT + front + back, shaftD / 2 - fit / 2).translate(x, 0, -plateT - front), { ...steel, process: "lathe" });
parts.push(shaft("Input shaft", xIn, 25, 0.5), shaft("Intermediate shaft", xMid, 0.5, 0.5), shaft("Output shaft", xOut, 0.5, 30));

// Deep-groove ball bearings, pressed into the plates (bought in).
const ring = (od, id, w) => cylinder(w, od / 2).subtract(cylinder(w + 2, id / 2).translate(0, 0, -1));
for (const [name, x] of [["input", xIn], ["intermediate", xMid], ["output", xOut]]) {
  parts.push(part(`Front bearing, ${name}`, ring(bearing.D, bearing.d, bearing.B).translate(x, 0, -plateT), { ...steel, process: "buy" }));
  parts.push(part(`Back bearing, ${name}`, ring(bearing.D, bearing.d, bearing.B).translate(x, 0, zBack + plateT - bearing.B), { ...steel, process: "buy" }));
}

// Plates: one outline around all three shafts, bearing bores, four corner holes.
const margin = pitchRadius(s2.m, s2.z2) + s2.m * 2 + 6;
const left = xIn - pitchRadius(s1.m, s1.z1) - s1.m - 14;
const outline = polygon([[left, -margin], [xOut + margin, -margin], [xOut + margin, margin], [left, margin]]);
const corners = [[left + 8, -margin + 8], [xOut + margin - 8, -margin + 8], [xOut + margin - 8, margin - 8], [left + 8, margin - 8]];
const plate = (z) => {
  let profile = outline.offset(-6).offset(6); // rounded corners
  const holes = [xIn, xMid, xOut].map((x) => circle(bearing.D / 2 + fit / 2).translate(x, 0));
  for (const [x, y] of corners) holes.push(circle(p.bolt / 2 + 0.2).translate(x, y));
  return profile.subtract(...holes).extrude(plateT).translate(0, 0, z);
};
parts.push(part("Front plate", plate(-plateT), { material: "Aluminium", process: "laser" }));
parts.push(part("Back plate", plate(zBack), { material: "Aluminium", process: "laser" }));

// Spacers hold the plates apart, on the corner bolts.
corners.forEach(([x, y], i) => {
  parts.push(part(`Spacer ${i + 1}`, ring(p.bolt * 2, p.bolt + 0.4, inner).translate(x, y, 0), { material: "Aluminium", process: "lathe" }));
});

// How it moves: each shaft turns with its gears; the plates, bearings and
// spacers stay put. The input pinion drives the stage-1 wheel, whose shaft's
// pinion drives the output wheel.
hinge(["Input shaft", "Input pinion"], { at: [xIn, 0, 0], axis: [0, 0, 1] });
hinge(["Intermediate shaft", "Stage 1 wheel", "Intermediate pinion"], { at: [xMid, 0, 0], axis: [0, 0, 1] });
hinge(["Output shaft", "Output wheel"], { at: [xOut, 0, 0], axis: [0, 0, 1] });
mesh("Input pinion", "Stage 1 wheel", { module: s1.m, teeth: [s1.z1, s1.z2], width: p.width1, backlash: p.backlash });
mesh("Intermediate pinion", "Output wheel", { module: s2.m, teeth: [s2.z1, s2.z2], width: p.width2, backlash: p.backlash });

return parts;
