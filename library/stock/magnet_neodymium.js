// Neodymium (NdFeB) permanent magnet, grade N52, nickel plated (Ni-Cu-Ni),
// magnetised through its thickness (axially): disc, ring, or disc with a
// countersunk hole for a flat head screw.
//
// Source: the makers' standard sizes (K&J Magnetics, supermagnete, the
// common AliExpress N52 listings): outside diameter D, thickness T, ring
// bore; countersunk magnets take an ISO 10642 screw: a clearance hole
// and a 90° countersink just over the screw's head (M3 Ø3.4 / Ø6.9, M4
// Ø4.5 / Ø9.2, M5 Ø5.5 / Ø11.4). Edges broken 0.1 mm, as they come.
// Surface field: on the axis at the face centre, from the field of a
// uniformly magnetised cylinder, Bz = Br/2 · (T/√(T² + R²) − T/√(T² + r²))
// with the remanence Br = 1.43 T (N52 typical) and r the bore radius (0 for
// a disc).
//
// Lies on its back face at z = 0, its front face (north pole) at z = T,
// axis along +Z.
// Mass: this nominal geometry at 7.5 g/cm³ (sintered NdFeB).

const CSK = {
  //  hole   countersink
  3: [3.4, 6.9],
  4: [4.5, 9.2],
  5: [5.5, 11.4],
};

const kind = { 1: "disc", 2: "ring", 3: "countersunk" }[p.kind];
if (!kind) throw new Error("kind: 1 = disc, 2 = ring, 3 = countersunk");
const D = p.D;
const T = p.T;
const R = D / 2;
const [hole, csk] = kind === "countersunk" ? lookup(CSK, p.hole, "countersunk magnet screw") : [kind === "ring" ? p.hole : 0, 0];
const r = hole / 2;
const c = 0.1; // edge break

// The section revolved: outer edges broken; a ring's bore edges too; a
// countersunk magnet's bore opens in a 90° cone at the front face.
const outer = [[R - c, 0], [R, c], [R, T - c], [R - c, T]];
let pts;
if (kind === "disc") pts = [[0, 0], ...outer, [0, T]];
else if (kind === "ring") pts = [[r + c, 0], ...outer, [r + c, T], [r, T - c], [r, c]];
else {
  const depth = Math.min((csk - hole) / 2, T - 0.5);
  pts = [[r + c, 0], ...outer, [r + depth, T], [r, T - depth], [r, c]];
}
const magnet = polygon(pts).revolve(360);

const Br = 1.43;
const field = (Br / 2) * (T / Math.hypot(T, R) - (r > 0 ? T / Math.hypot(T, r) : 0));
const massG = 7.5e-3 * magnet.volume;
const vol = magnet.volume;
const name = kind === "disc" ? `Ø${num(D)}×${num(T)}` : kind === "ring" ? `${num(D)}×${num(hole)}×${num(T)}` : `Ø${num(D)}×${num(T)} M${num(p.hole)}`;
const words = { disc: "disc", ring: "ring", countersunk: `countersunk disc for M${num(p.hole)}` }[kind];
stock({
  partNumber: `NdFeB N52 ${kind === "ring" ? "ring " : kind === "countersunk" ? "CS " : ""}${name} NiCuNi`,
  description: `Neodymium magnet N52 ${words} ${name} mm, axially magnetised, nickel plated: about ${Math.round(field * 1000)} mT at the face`,
  unitPrice: Math.round((0.03 + 0.0008 * vol + (kind === "countersunk" ? 0.1 : 0)) * 100) / 100,
  currency: "USD",
  pack: vol < 150 ? 50 : 10,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "magnets"),
    supplierSearch("Misumi", `neodymium magnet ${num(D)}x${num(T)}`),
    supplierSearch("Amazon", `N52 neodymium ${words} magnet ${num(D)}x${num(T)}mm`),
    supplierSearch("AliExpress", `N52 neodymium magnet ${kind === "disc" ? "" : kind + " "}${num(D)}x${num(T)}${kind === "ring" ? `-${num(hole)}` : ""}mm`),
  ],
});
metric("Diameter", D, "mm");
metric("Thickness", T, "mm");
metric("Surface field", Math.round(field * 1000), "mT");
if (r > 0) metric("Bore Ø", hole, "mm");

// The front face (north) and the back, the axis the field runs along.
connector("Face", { at: [0, 0, T], axis: [0, 0, 1] });
connector("Back face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
return magnet;
