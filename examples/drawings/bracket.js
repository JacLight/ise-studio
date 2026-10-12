// A machined bearing bracket from 6061 plate: a base with a pocket, four
// M6 tapped holes to bolt things on, two dowel holes for location, a
// counterbored hole for the screw that holds it down, and an upright flange
// carrying a bored seat for a bearing.
const L = p.length, W = p.width, T = p.base, F = p.flange, H = p.height;
const base = box(L, W, T);
const flange = box(F, W, H - T).translate(0, 0, T);
// The pocket: rounded corners the cutter leaves (its radius).
const [px0, px1, py0, py1] = [35, 95, 20, 60];
const r = p.pocketRadius;
const pocket = rect(px1 - px0 - 2 * r, py1 - py0 - 2 * r).offset(r).extrude(p.pocketDepth + 1).translate(px0 + r, py0 + r, T - p.pocketDepth);
// Tapped M6 holes: the tap drill ⌀5, 12 deep from the top of the base.
const tapped = [[25, 10], [110, 10], [25, 70], [110, 70]].map(([x, y]) => cylinder(12 + 1, 2.5).translate(x, y, T - 12));
// Dowel holes through, reamed (⌀8 H7).
const dowels = [[65, 10], [65, 70]].map(([x, y]) => cylinder(T + 2, 4).translate(x, y, -1));
// An ISO 4762 M6 counterbore: ⌀6.6 through, ⌀11 × 6.5 deep.
const cbore = [cylinder(T + 2, 3.3).translate(110, 40, -1), cylinder(6.5 + 1, 5.5).translate(110, 40, T - 6.5)];
// The bearing seat through the flange, along X.
const seat = cylinder(F + 2, p.bore / 2).rotate(0, 90, 0).translate(-1, W / 2, p.boreHeight);
return base.add(flange).subtract(pocket, ...tapped, ...dowels, ...cbore, seat);
