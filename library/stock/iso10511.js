// Prevailing torque hexagon nut with a nylon insert ("nyloc"), thin type,
// ISO 10511 (DIN 985), stainless A2-70 with a PA66 ring.
//
// Source: ISO 10511:2012 table 1 — width across flats s (nominal; M10 16,
// M12 18 where DIN 985 had 17/19), overall height h (max), height of the
// hex (wrenching) part ≈ m min; M2.5, which ISO 10511 does not list, from
// DIN 985. The crimped collar that holds the ring: about 0.9 s across, its
// lip turned in over the ring, as the makers draw it. The ring's bore is a
// little under the thread's minor diameter, so the screw cuts into it.
//
// Stands on its bearing face at z = 0 (hex, 30° chamfer underneath), the
// collar on top up to z = h, axis along +Z.
// Mass: this nominal geometry, the nut at 7.9 g/cm³ and the ring at 1.14.

const ISO10511 = {
  //     s     h    hex
  2.5: [5.0, 3.8, 2.0],
  3: [5.5, 4.0, 2.4],
  4: [7.0, 5.0, 2.9],
  5: [8.0, 5.0, 3.2],
  6: [10.0, 6.0, 4.0],
  8: [13.0, 8.0, 5.5],
  10: [16.0, 10.0, 6.5],
  12: [18.0, 12.0, 8.0],
};
// Typical price of one nut in a pack of 100 (stainless A2, USD).
const PRICE = { 2.5: 0.05, 3: 0.04, 4: 0.045, 5: 0.05, 6: 0.065, 8: 0.11, 10: 0.17, 12: 0.25 };

const d = p.d;
const [s, h, mh] = lookup(ISO10511, d, "ISO 10511");
const pitch = coarsePitch(d);
const collarR = 0.45 * s;
const lip = Math.max(0.3, 0.06 * d); // the crimped-over lip's thickness
const wall = Math.max(0.35, 0.05 * s); // collar wall round the ring
const ringR = collarR - wall;

// Hex: 30° chamfer on the bearing face (cone from 0.95 s across).
const r0 = 0.475 * s;
const hex = hexPrism(s, mh).intersect(cylinder(mh, r0, { top: r0 + mh * Math.tan(rad(60)) }));
// Collar from the hex's top to h, its top edge rounded over (a 45° bevel the
// size of the lip).
const collar = polygon([[0, mh - 0.01], [collarR, mh - 0.01], [collarR, h - lip], [collarR - lip, h], [0, h]]).revolve(360);
const sink = 0.5 * pitch;
const cs = d / 2 + 0.05;
const nut = union(hex, collar)
  .subtract(internalThreadTool({ d, pitch, z0: 0, z1: mh, real: p.threads >= 0.5 }))
  .subtract(cylinder(sink + 0.01, cs, { top: cs - sink - 0.01 }).translate(0, 0, -0.01))
  .subtract(cylinder(h - lip - mh, ringR).translate(0, 0, mh)) // the ring's pocket
  .subtract(cylinder(h, d / 2 + 0.05).translate(0, 0, mh)); // through the lip
// The nylon ring filling the pocket.
const ringBore = d - 1.0825 * pitch - 0.1; // 0.1 mm under the minor diameter
const ring = polygon([[ringBore / 2, mh], [ringR, mh], [ringR, h - lip], [ringBore / 2, h - lip]]).revolve(360);

const massG = 7.9e-3 * nut.volume + 1.14e-3 * ring.volume;
stock({
  standard: "ISO 10511",
  partNumber: `ISO 10511 M${num(d)} A2-70`,
  description: `Nylon insert lock nut (nyloc) M${num(d)}, ${num(s)} mm across flats, ${num(h)} mm high, stainless A2-70`,
  unitPrice: lookup(PRICE, d, "price"),
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "nylon insert locknuts"),
    supplierSearch("Bolt Depot", `M${num(d)} nylon insert lock nut stainless`),
    supplierSearch("Misumi", `nylon lock nut M${num(d)} stainless`),
    supplierSearch("AliExpress", `M${num(d)} nylon lock nut DIN 985 A2 stainless`),
  ],
});
metric("Across flats", s, "mm");
metric("Height", h, "mm");
metric("Hex height", mh, "mm");

connector("Bottom face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top face", { at: [0, 0, h], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
return [part("Nut", nut, { material: "Stainless steel A2" }), part("Nylon ring", ring, { material: "Nylon PA66" })];
