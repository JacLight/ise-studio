// Hex nut, ISO 4032 style, chamfered 30° on both faces, with an internal
// thread cut by the same ISO profile grown by the clearance.
const R = p.diameter / 2 + p.clearance;
const hex = regularPolygon(6, p.across / Math.sqrt(3)).extrude(p.height);
const r0 = (p.across / 2) * 0.96;
const rise = p.height / Math.tan(Math.PI / 6);
const lower = cylinder(p.height, r0, { top: r0 + rise });
const upper = cylinder(p.height, r0 + rise, { top: r0 });
const body = hex.intersect(lower).intersect(upper);

// Thread phase is anchored at the nut's bottom face (z = 0).
const tool = threadedRod({ majorRadius: R, pitch: p.pitch, z0: -p.pitch, z1: p.height + p.pitch, phase: 0 });
return body.subtract(tool);
