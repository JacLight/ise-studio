// Hex bolt, ISO 4017 style: hex head at the bottom (z = 0 … head), threaded
// shank above it with a 45° chamfer at the tip.
const R = p.diameter / 2 - p.clearance; // external thread sits slightly under basic size
const H = (Math.sqrt(3) / 2) * p.pitch;
const minor = R - (5 * H) / 8;
const top = p.headHeight + p.length;

// Thread phase is anchored at the head face, so a nut placed at headHeight + n·pitch engages exactly.
const rod = threadedRod({ majorRadius: R, pitch: p.pitch, z0: p.headHeight - 0.01, z1: top, phase: p.headHeight });
// 45° tip chamfer down to just under the minor diameter: keep everything inside
// a full-length envelope whose last stretch is a cone.
const cone = R + 1 - (minor - 0.15); // axial length of the 45° cone
const envelope = cylinder(top - cone, R + 1).add(
  cylinder(cone, R + 1, { top: minor - 0.15 }).translate(0, 0, top - cone),
);
const shank = rod.intersect(envelope);

// Hex head with the usual 30° chamfer on its outer face.
const hex = regularPolygon(6, p.headAcross / Math.sqrt(3)).extrude(p.headHeight);
const chamfer = cylinder(p.headHeight, (p.headAcross / 2) * 0.96, { top: (p.headAcross / 2) * 0.96 + p.headHeight / Math.tan(Math.PI / 6) });
const head = hex.intersect(chamfer);

return head.add(shank);
