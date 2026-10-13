// A fender-like crown panel with a four-sided hole, refilled G1 to its neighbours.
const crown = (x, y) => 20 - (x * x + 1.5 * y * y) / 250 + 0.0004 * x * x * x;
const line = (f) => spline(Array.from({ length: 11 }, (_, k) => f(-60 + 12 * k)));
const ys = [-60, -24, 24, 60];
const panel = networkSurface(ys.map((y) => line((x) => [x, y, crown(x, y)])), ys.map((x) => line((y) => [x, y, crown(x, y)])));
const holed = panel.trimWith(box(40, 30, 200, { center: true }), { keep: [-55, -55, crown(-55, -55)] });
const patch = fillSurface([holed.border({ near: [0, 0, 20] }, { continuity: 1 })]);
const seam = continuity(patch, holed);
metric("seam gap", seam.max.g0, "mm");
metric("seam crease", seam.max.g1, "deg");
metric("seam curvature jump", seam.max.g2, "1/mm");
const skin = sew([holed, patch], { solid: false });
return [part("Fender", skin.thicken(1, { join: "round" }), { material: "steel" })];
