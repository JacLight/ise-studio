// Hexagon socket button head screw, ISO 7380-1, stainless A2-70.
//
// Source: ISO 7380-1:2011 table 1 — head diameter dk (max), head height k
// (max), socket across flats s (nominal), socket depth t (min); ISO 261
// coarse pitch. The catalogue lengths (up to 30 mm here) are threaded to
// the head.
//
// The head is a dome: a spherical cap over a short cylindrical edge (about
// a fifth of k), as the standard draws it. Stands with the head's bearing
// face on z = 0, head up +Z, shank down −Z.
// Mass: the ISO nominal geometry at 7.9 g/cm³ (stainless).

const ISO7380 = {
  //    dk     k    s     t
  3: [5.7, 1.65, 2.0, 1.04],
  4: [7.6, 2.2, 2.5, 1.3],
  5: [9.5, 2.75, 3.0, 1.56],
  6: [10.5, 3.3, 4.0, 2.08],
  8: [14.0, 4.4, 5.0, 2.6],
};
// Typical price of one screw in a pack of 100 (stainless A2, USD).
const PRICE = { 3: [0.045, 0.0025], 4: [0.055, 0.003], 5: [0.065, 0.004], 6: [0.09, 0.005], 8: [0.15, 0.008] };

const d = p.d;
const L = p.length;
const [dk, k, s, t] = lookup(ISO7380, d, "ISO 7380");
const pitch = coarsePitch(d);

// Dome: the sphere through the top centre (0, k) and the edge (dk/2, e).
const e = 0.2 * k;
const rise = k - e;
const R = ((dk / 2) ** 2 + rise * rise) / (2 * rise);
const head = cylinder(k, dk / 2)
  .intersect(sphere(R).translate(0, 0, k - R))
  .subtract(hexSocket(s, t, k));
const screw = head.add(externalThread({ d, pitch, z0: -L, z1: 0.01, real: p.threads >= 0.5, tip: "bottom" }));

// Mass from the nominal geometry: the dome (by slices), less the socket,
// plus the shank at the pitch diameter.
const rho = 7.9e-3;
let dome = 0;
const n = 200;
for (let i = 0; i < n; i++) {
  const z = ((i + 0.5) / n) * k;
  const rz = Math.min(dk / 2, Math.sqrt(Math.max(0, R * R - (z - (k - R)) ** 2)));
  dome += Math.PI * rz * rz * (k / n);
}
const d2 = d - 0.6495 * pitch;
const socketVol = (3 * Math.sqrt(3) / 2) * (s / Math.sqrt(3)) ** 2 * t;
const massG = rho * (dome - socketVol + (Math.PI / 4) * d2 * d2 * L);
const [base, perMm] = lookup(PRICE, d, "price");
const size = `M${num(d)}×${num(L)}`;
stock({
  standard: "ISO 7380-1",
  partNumber: `ISO 7380-1 ${size} A2-70`,
  description: `Button head socket screw ${size}, stainless A2-70, ${num(s)} mm hex key`,
  unitPrice: Math.round((base + perMm * L) * 1000) / 1000,
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "button head socket cap screws"),
    supplierSearch("Bolt Depot", `M${num(d)} x ${num(L)} button head socket cap screw stainless`),
    supplierSearch("Misumi", `button head socket screw M${num(d)} ${num(L)}`),
    supplierSearch("AliExpress", `M${num(d)} button head socket screw ISO 7380 A2 ${num(L)}mm`),
  ],
});
metric("Hex key", s, "mm");
metric("Thread length", L, "mm");

connector("Head seat", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Axis", { at: [0, 0, -L], axis: [0, 0, 1] });
connector("Socket", { at: [0, 0, k], axis: [0, 0, 1] });
return screw;
