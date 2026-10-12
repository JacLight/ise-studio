// MGN miniature linear guide rail (MGN7R, MGN9R, MGN12R, MGN15R), hardened
// bearing steel, cut to length with its counterbored mounting holes.
//
// Source: HIWIN MG series rail table (see mgn.js). Hole pattern: the holes
// on pitch P, the end distance E the same at both ends (HIWIN's standard
// rail lengths L = (n − 1)·P + 2E; E at least the counterbore radius
// plus 1 mm). Mass: the catalogue kg/m times the length.
//
// Lies along +X from x = 0 to L, base on z = 0, centred on y = 0. The
// "Slide" connector at the rail's start runs along it: join a block's
// "Slide" to it with a slider joint and the joint's value is where the
// block's centre is along the rail.

const s = mgn(p.series);
const L = p.length;
const { yb, zb, rg } = mgnBallRows(s);
const c = 0.3; // top corner chamfers

// Cross-section in (y, z): the bar with its chamfered top corners and the
// two gothic-arch ball grooves at the upper shoulders.
const section = polygon([
  [-s.WR / 2, 0],
  [s.WR / 2, 0],
  [s.WR / 2, s.HR - c],
  [s.WR / 2 - c, s.HR],
  [-s.WR / 2 + c, s.HR],
  [-s.WR / 2, s.HR - c],
]).subtract(circle(rg).translate(yb, zb), circle(rg).translate(-yb, zb));
let rail = alongX(section.extrude(L));

// Mounting holes: counterbore D × h from the top, through hole d.
const Emin = s.Dc / 2 + 1;
const n = Math.floor((L - 2 * Emin) / s.P) + 1;
const E = (L - (n - 1) * s.P) / 2;
const xs = [];
for (let i = 0; i < n; i++) xs.push(E + i * s.P);
const hole = cylinder(s.HR + 0.2, s.dh / 2).translate(0, 0, -0.1).add(cylinder(s.hc + 0.1, s.Dc / 2).translate(0, 0, s.HR - s.hc));
rail = rail.subtract(union(xs.map((x) => hole.translate(x, 0, 0))));

const name = `MGN${s.WR}R`;
stock({
  standard: "HIWIN MGN (MG series)",
  partNumber: `${name}-${num(L)}`,
  description: `Miniature linear guide rail ${name}, ${num(L)} mm, ${n} holes for M${s.bolt} on ${s.P} mm pitch, E = ${num(E)} mm`,
  manufacturer: "HIWIN (or MGN-pattern equivalent)",
  mpn: `${name}-${num(L)}-${num(E)}-${num(E)}`,
  unitPrice: cents({ 7: 4, 9: 5, 12: 6, 15: 8 }[s.WR] + { 7: 0.03, 9: 0.035, 12: 0.04, 15: 0.06 }[s.WR] * L),
  currency: "USD",
  pack: 1,
  massG: Math.round(s.kgPerM * L * 10) / 10,
  sources: [
    supplierSearch("Misumi", `miniature linear guide rail ${s.WR}mm ${num(L)}`),
    supplierSearch("McMaster-Carr", "ball bearing carriages and guide rails"),
    supplierSearch("AliExpress", `MGN${s.WR} linear rail ${num(L)}mm`),
    supplierSearch("Amazon", `MGN${s.WR} linear rail ${num(L)}mm`),
  ],
});
metric("Length", L, "mm");
metric("Rail width", s.WR, "mm");
metric("Rail height", s.HR, "mm");
metric("Hole pitch", s.P, "mm");
metric("End distance", E, "mm");
metric("Holes", n);
metric("Bolt", s.bolt, "mm");

connector("Slide", { at: [0, 0, 0], axis: [1, 0, 0], x: [0, 0, 1] });
connector("Base", { at: [L / 2, 0, 0], axis: [0, 0, -1] });
connector("Top", { at: [L / 2, 0, s.HR], axis: [0, 0, 1] });
connector("End A", { at: [0, 0, s.HR / 2], axis: [-1, 0, 0], x: [0, 0, 1] });
connector("End B", { at: [L, 0, s.HR / 2], axis: [1, 0, 0], x: [0, 0, 1] });
// Each hole at its counterbore floor, where a screw head seats.
xs.forEach((x, i) => connector(`Hole ${i + 1}`, { at: [x, 0, s.HR - s.hc], axis: [0, 0, 1] }));
return rail;
