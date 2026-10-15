// A two-stage 1 : 7.5 reducer that prints in one piece and turns straight
// off the bed, with nothing for the slicer to support:
//
// - Every gear stands on the bed. The housing is a frame with no floor, so no
//   face is ever printed over a gap or over air.
// - The stage-2 wheel sits beside the stage-1 wheel instead of above it (its
//   pinion is nearly as big as the stage-1 wheel); it overhangs that wheel
//   only by a sliver at the rim.
// - Herringbone (V) teeth lock each gear to its mate along the axis, so the
//   train cannot slide apart. V teeth at 30° print without supports.
// - The input hangs from a bridge across the top by the 45° cone under its
//   knob, so lifting the housing lifts the whole train with it.
// - Every bottom edge is chamfered, so the first layer's squash cannot weld
//   neighbouring parts together.

const tip = (m, z) => pitchRadius(m, z) + m;
const root = (m, z) => pitchRadius(m, z) - 1.25 * m;
const c = p.clearance; // running clearance: gear tips to walls, teeth to teeth
const g = p.layerGap; // between faces printed one over the other
const bevel = p.chamfer; // on every bottom edge
const s1 = { m: p.module1, z1: p.pinion1, z2: p.gear1 };
const s2 = { m: p.module2, z1: p.pinion2, z2: p.gear2 };
const a1 = pitchRadius(s1.m, s1.z1) + pitchRadius(s1.m, s1.z2) + p.meshGap;
const a2 = pitchRadius(s2.m, s2.z1) + pitchRadius(s2.m, s2.z2) + p.meshGap;
const xIn = 0, xMid = a1, xOut = a1 + a2;
const w1 = p.width1, w2 = p.width2;
const top = w1 + g + w2;

// How far the stage-2 wheel reaches over the stage-1 wheel (it must be small).
const lens = tip(s1.m, s1.z2) + tip(s2.m, s2.z2) - a2;
if (lens > 6) throw new Error(`the stage-2 wheel would overhang the stage-1 wheel by ${lens.toFixed(1)} mm: use a bigger stage-2 pinion`);

/**
 * A herringbone gear: the involute outline twisted one way to mid-height and
 * back, with a helix angle of `p.helix`. `hand` (±1) is opposite on mating gears.
 */
function herringbone(m, z, height, hand) {
  const r = pitchRadius(m, z);
  const twist = (((height / 2) * Math.tan(rad(p.helix))) / r) * (180 / Math.PI) * hand;
  const slices = Math.max(2, Math.ceil(Math.abs(twist) / 1.5));
  const outline = gearOutline({ m, z, backlash: p.backlash });
  return outline
    .extrude(height / 2, { twist, slices })
    .add(outline.rotate(twist).extrude(height / 2, { twist: -twist, slices }).translate(0, 0, height / 2));
}

/** Cuts a 45° chamfer `bevel` deep round the bottom of a body of outer radius r. */
const chamferBottom = (solid, r) => solid.intersect(cylinder(top + 2, r - bevel, { top: r - bevel + top + 2 }));

// ---- The rotating bodies -------------------------------------------------------------------

// Input: stage-1 pinion, then a column up through the bridge to the knob. The
// column passes the stage-2 pinion, so the knob flares out (45°) only above
// the housing's top, a layer gap over the bridge it hangs from.
const columnR = Math.min(root(s1.m, s1.z1), a1 - tip(s2.m, s2.z1) - c) - 0.2;
const bridgeBottom = w1 + p.bridgeGap;
const z0 = top + g;
const knobR = 16;
const knobH = 6;
const flare = knobR - columnR;
let knob = cylinder(flare, columnR, { top: knobR }).add(cylinder(knobH, knobR).translate(0, 0, flare - 0.01));
for (let i = 0; i < 18; i++) {
  const a = (i / 18) * 2 * Math.PI;
  knob = knob.subtract(cylinder(flare + knobH + 2, 1.2).translate(knobR * Math.cos(a), knobR * Math.sin(a), -1));
}
knob = knob.add(cylinder(10, 2.5).translate(knobR - 5, 0, flare + knobH - 0.02)); // crank handle
const input = chamferBottom(herringbone(s1.m, s1.z1, w1, 1), tip(s1.m, s1.z1))
  .add(cylinder(z0 - w1 + 0.02, columnR).translate(0, 0, w1 - 0.01))
  .add(knob.translate(0, 0, z0))
  .translate(xIn, 0, 0);

// Intermediate: stage-1 wheel on the bed, stage-2 pinion standing on it.
const middle = chamferBottom(herringbone(s1.m, s1.z2, w1, -1).rotate(0, 0, 180 + 180 / s1.z2), tip(s1.m, s1.z2))
  // A disc lifts the stage-2 pinion one layer gap: its teeth reach over the input pinion's top.
  .add(cylinder(g + 0.01, Math.min(root(s2.m, s2.z1), a1 - tip(s1.m, s1.z1) - c)).translate(0, 0, w1 - 0.005))
  .add(herringbone(s2.m, s2.z1, w2, 1).translate(0, 0, w1 + g))
  .translate(xMid, 0, 0);

// Output: a disc on the bed clear of the stage-1 wheel, the stage-2 wheel on
// it (overhanging only the sliver `lens`), and a pointer on top.
const wheelR = tip(s2.m, s2.z2);
const discR = a2 - tip(s1.m, s1.z2) - c;
const output = chamferBottom(cylinder(w1 + g + 0.01, discR), discR)
  .add(herringbone(s2.m, s2.z2, w2, -1).rotate(0, 0, 180 + 180 / s2.z2).translate(0, 0, w1 + g))
  .add(cylinder(4, 6).translate(0, 0, top - 0.01))
  .add(box(wheelR * 0.62, 4, 2).translate(5, -2, top - 0.01))
  .add(cylinder(2, 4, { top: 0.01 }).rotate(0, 90, 0).translate(5 + wheelR * 0.62 - 0.01, 0, top + 1))
  .translate(xOut, 0, 0);

// ---- The housing: a floorless frame, pockets straight through ------------------------------

/** A pocket of radius r, flared at the bottom to match the bodies' chamfers. */
const pocket = (x, r) => cylinder(top + 2, r).translate(x, 0, -1).add(cylinder(bevel + 1, r + bevel + 1, { top: r }).translate(x, 0, -1));
const pockets = [pocket(xIn, tip(s1.m, s1.z1) + c), pocket(xMid, tip(s1.m, s1.z2) + c), pocket(xOut, wheelR + c)];
const margin = Math.max(tip(s1.m, s1.z2), wheelR) + c + p.wall;
const left = xIn - Math.max(tip(s1.m, s1.z1) + c + p.wall, knobR + 2);
const right = xOut + wheelR + c + p.wall;
const outline = polygon([[left, -margin], [right, -margin], [right, margin], [left, margin]]).offset(-8).offset(8);
const corners = [[left + 7, -margin + 7], [right - 7, -margin + 7], [right - 7, margin - 7], [left + 7, margin - 7]];
let housing = outline.subtract(corners.map(([x, y]) => circle(2.2).translate(x, y))).extrude(top).subtract(pockets);

// The bridge across the input pocket, clear of the stage-2 pinion and the
// column; a layer gap under the knob's cone, and room above the pinion.
const span = tip(s1.m, s1.z1) + c + p.wall;
const bridge = box(2 * span, 2 * span, top - bridgeBottom)
  .translate(xIn - span, -span, bridgeBottom)
  .subtract(cylinder(top + 2, tip(s2.m, s2.z1) + c).translate(xMid, 0, -1))
  .subtract(cylinder(top + 2, columnR + c).translate(xIn, 0, -1))
  .intersect(outline.extrude(top));
housing = housing.add(bridge);

// ---- Check the print will turn -------------------------------------------------------------

const bodies = [["Input", input], ["Intermediate gears", middle], ["Output wheel", output], ["Housing", housing]];
let smallest = Infinity;
for (let i = 0; i < bodies.length; i++)
  for (let j = i + 1; j < bodies.length; j++) {
    const d = bodies[i][1].minGap(bodies[j][1], 2);
    if (d < 0.2) throw new Error(`${bodies[i][0]} and ${bodies[j][0]} are only ${d.toFixed(2)} mm apart: they would print fused`);
    smallest = Math.min(smallest, d);
  }
metric("Smallest gap between moving parts", Math.round(smallest * 100) / 100, "mm");
metric("Reduction", Math.round(((s1.z2 / s1.z1) * (s2.z2 / s2.z1)) * 100) / 100, ": 1");
metric("Stage-2 wheel over stage-1 wheel", Math.round(Math.max(0, lens) * 10) / 10, "mm");

// How it moves: three bodies turn in the housing; the knob drives.
hinge("Input", { at: [xIn, 0, 0], axis: [0, 0, 1] });
hinge("Intermediate gears", { at: [xMid, 0, 0], axis: [0, 0, 1] });
hinge("Output wheel", { at: [xOut, 0, 0], axis: [0, 0, 1] });
mesh("Input", "Intermediate gears", { module: s1.m, teeth: [s1.z1, s1.z2], width: w1, backlash: p.backlash });
mesh("Intermediate gears", "Output wheel", { module: s2.m, teeth: [s2.z1, s2.z2], width: w2, backlash: p.backlash });

const print = { material: "PLA", process: "3d_print" };
return [
  part("Housing", housing, print),
  part("Input", input, print),
  part("Intermediate gears", middle, print),
  part("Output wheel", output, print),
];
