// A fluted vase skinned through a network of sections and guides.
const zs = [0, 50, 110, 170, 215];
const rs = [36, 56, 38, 25, 34];
const flute = [0, 0.05, 0.08, 0.04, 0.02];   // depth of the six flutes, per section
const n = 24, lobes = 6;
const at = (k, t) => {
  const r = rs[k] * (1 + flute[k] * Math.cos(lobes * t));
  return [r * Math.cos(t), r * Math.sin(t), zs[k]];
};
const profiles = zs.map((_, k) => spline(Array.from({ length: n }, (_, i) => at(k, (2 * Math.PI * i) / n)), { closed: true }));
const guides = Array.from({ length: 12 }, (_, j) => spline(zs.map((_, k) => at(k, (2 * Math.PI * j) / 12))));
const wall = networkSurface(profiles, guides);
metric("network gap", wall.report.gap, "mm");
metric("network deviation", wall.report.deviation, "mm");
const floor = fillSurface([wall.border({ near: [rs[0], 0, 0] }, { continuity: 0 })]);
const cup = sew([wall, floor], { solid: false });
metric("open edges", cup.report.freeEdges);
const vase = cup.thicken(2.5, { join: "round" });
return [part("Vase", vase, { material: "white PLA" })];
