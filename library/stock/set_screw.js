// Hexagon socket set screw (grub screw), steel 45H black oxide: the shape
// ISO 4026 (flat point), ISO 4027 (cone point) and ISO 4029 (cup point)
// share. The standard's own file runs first and sets POINT ("flat", "cone"
// or "cup"), STANDARD and POINT_D (its table of point diameters).
//
// Source: ISO 4026/4027/4029:2003 table 1 — socket across flats s
// (nominal) and socket depth t (min; the shorter figure for the shortest
// lengths, l ≤ 1.5 d); the socket end chamfered 45° to just under the
// thread's minor diameter; ISO 261 coarse pitch.
//
// Stands with the socket end flush at z = 0 (the surface it is screwed in
// to), the point down −Z at z = −l.
// Mass: this nominal geometry at 7.85 g/cm³ (steel).

const SET_SOCKET = {
  //   s   t short  t long
  3: [1.5, 0.7, 1.2],
  4: [2.0, 0.8, 1.5],
  5: [2.5, 1.2, 2.0],
  6: [3.0, 1.2, 2.0],
  8: [4.0, 1.5, 3.0],
};
// Typical price of one screw in a pack of 100 (steel 45H, USD): base + per mm.
const SET_PRICE = { 3: [0.03, 0.002], 4: [0.035, 0.0025], 5: [0.04, 0.003], 6: [0.05, 0.004], 8: [0.08, 0.006] };

const d = p.d;
const L = p.length;
const [s, tShort, tLong] = lookup(SET_SOCKET, d, STANDARD);
const t = L <= 1.5 * d ? tShort : tLong;
const pitch = coarsePitch(d);
const dp = lookup(POINT_D, d, STANDARD);
const R = d / 2 + 0.5; // past the crests: the envelope's cones start outside the thread

// The threaded rod with the 45° chamfer at the socket end.
const rod = externalThread({ d, pitch, z0: -L, z1: 0, real: p.threads >= 0.5, tip: "top" });
// The point: a 45° cone (90° included) from the point diameter at the tip —
// the flat point's chamfer, the cone point's cone (to a small flat dt), the
// cup's outer edge.
const point = polygon([[0, -L], [dp / 2, -L], [R, -L + (R - dp / 2)], [R, 0.5], [0, 0.5]]).revolve(360);
let screw = rod.intersect(point).subtract(hexSocket(s, t, 0));
if (POINT === "cup") {
  // The cup: a 90° conical recess three quarters of the point's diameter across.
  const rc = 0.375 * dp;
  screw = screw.subtract(cylinder(rc + 0.01, rc + 0.01, { top: 0.0001 }).translate(0, 0, -L - 0.01));
}

const massG = 7.85e-3 * screw.volume;
const [base, perMm] = lookup(SET_PRICE, d, "price");
const size = `M${num(d)}×${num(L)}`;
const words = { flat: "flat point", cone: "cone point", cup: "cup point" }[POINT];
stock({
  standard: STANDARD,
  partNumber: `${STANDARD} ${size} 45H`,
  description: `Hex socket set screw ${size}, ${words}, steel 45H black oxide, ${num(s)} mm hex key`,
  unitPrice: Math.round((base + perMm * L) * 1000) / 1000,
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "set screws"),
    supplierSearch("Bolt Depot", `M${num(d)} x ${num(L)} socket set screw ${words}`),
    supplierSearch("Misumi", `set screw ${words} M${num(d)}x${num(L)}`),
    supplierSearch("AliExpress", `M${num(d)} ${num(L)}mm grub screw ${words} 45H`),
  ],
});
metric("Hex key", s, "mm");
metric("Point Ø", dp, "mm");

// The socket end (flush with the surface), the point that bears on the shaft,
// and the axis from the point up.
connector("Top face", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("Socket", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("Tip", { at: [0, 0, -L], axis: [0, 0, -1] });
connector("Axis", { at: [0, 0, -L], axis: [0, 0, 1] });
return screw;
