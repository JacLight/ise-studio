// A computer mouse shell: a lofted skirt and a plate-filled crown joined by
// a G2 blend, floored, sewn into a solid and hollowed.
const outline = (z, k) =>
  spline(Array.from({ length: 32 }, (_, i) => {
    const t = (2 * Math.PI * i) / 32;
    return [58 * k * Math.cos(t), 31 * k * (1 - 0.1 * Math.cos(t)) * Math.sin(t), z];
  }), { closed: true });
const skirt = sectionSurface([outline(0, 0.95), outline(7, 1), outline(14, 0.985)]);
// The crown's rim, inset and rising toward the palm.
const rim = spline(Array.from({ length: 32 }, (_, i) => {
  const t = (2 * Math.PI * i) / 32;
  return [46 * Math.cos(t), 24 * (1 - 0.1 * Math.cos(t)) * Math.sin(t), 25 - 3 * Math.cos(t)];
}), { closed: true });
const crown = fillSurface([rim], { points: [[-12, 0, 38], [15, 0, 34], [36, 0, 28], [-34, 0, 32], [-12, 17, 33], [-12, -17, 33], [15, 15, 31], [15, -15, 31]] });
metric("crown plate g0", crown.report.g0, "mm");
const shoulder = blendSurface(skirt.border({ near: [60, 0, 14] }, { continuity: 2 }), crown.border(undefined, { continuity: 2 }));
const floor = fillSurface([skirt.border({ near: [60, 0, 0] }, { continuity: 0 })]);
for (const [name, other] of [["skirt", skirt], ["crown", crown]]) {
  const c = continuity(shoulder, other);
  metric(`shoulder–${name} gap`, c.max.g0, "mm");
  metric(`shoulder–${name} crease`, c.max.g1, "deg");
  metric(`shoulder–${name} curvature jump`, c.max.g2, "1/mm");
}
const body = sew([floor, skirt, shoulder, crown]);
metric("closed", body.body.closed ? 1 : 0);
const shell = body.shell(1.8, { normal: [0, 0, -1] });
return [part("Mouse shell", shell, { material: "black ABS" })];
