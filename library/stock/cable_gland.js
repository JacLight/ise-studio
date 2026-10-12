// Cable gland, nylon PA66 (UL94 V-2), IP68, with NBR seal and locknut:
// PG (DIN 40430) or metric (EN 60423) entry thread.
//
// Source: the common PA66 gland ranges (Lapp SKINTOP-style, Jacob, and the
// generic "PG7 / M16 nylon cable gland" listings — within about 1 mm of
// each other): entry thread and its length, body and cap across flats,
// height above the panel, clamping range, panel hole, locknut. The PG
// thread's 80° flanks are drawn with the ISO 60° profile at the PG pitch
// (rings, like every thread here): it looks the same at this scale.
//
// The panel's outer face is z = 0: the flat flange seal on it, the body's
// hex flange, the cap nut with the clamping seal inside up +Z; the entry
// thread runs down −Z through the panel, the locknut drawn under a 2 mm
// panel.
// Mass: this nominal geometry, PA66 at 1.14 g/cm³, NBR at 1.25.

const GLANDS = {
  //  name     d     P    thread  AF   H   cable min max  hole  nut h
  1: ["PG7", 12.5, 1.27, 8, 15, 21, 3, 6.5, 12.7, 4],
  2: ["PG9", 15.2, 1.41, 8, 19, 22, 4, 8, 15.5, 4],
  3: ["PG11", 18.6, 1.41, 8, 22, 24, 5, 10, 19, 4.5],
  4: ["M12×1.5", 12, 1.5, 8, 15, 21, 3, 6.5, 12.2, 4],
  5: ["M16×1.5", 16, 1.5, 8, 19, 22, 4, 8, 16.2, 4.5],
  6: ["M20×1.5", 20, 1.5, 9, 24, 25, 6, 12, 20.3, 5],
  7: ["M25×1.5", 25, 1.5, 10, 30, 29, 13, 18, 25.3, 5],
};
// Typical price of one gland with its locknut in a pack of 10 (USD).
const PRICE = { 1: 0.22, 2: 0.25, 3: 0.3, 4: 0.22, 5: 0.25, 6: 0.32, 7: 0.45 };

const [name, d, pitch, tl, af, H, cMin, cMax, hole, nh] = lookup(GLANDS, p.size, "cable gland");
const panel = 2;
const fl = 1; // flange seal
const fh = 0.2 * H; // hex flange
const z1 = fl + fh;
const z2 = z1 + 0.6; // the cap's bottom (a little gap above the flange)
const zn = z2 + 0.35 * H; // the neck's top: the seal sits on it
const rn = 0.4 * af; // the neck the cap screws onto
const rTop = cMax / 2 + 1.2; // the cap's top, round the cable opening
const rs = Math.min(rn - 0.8, rTop); // the seal's outside (the cap's wall over it)
const bore = cMax / 2;

// Body: entry thread up through the flange seal, the hex flange with its
// top chamfered, the neck; bored for the largest cable.
const r0 = 0.475 * af;
const flange = hexPrism(af, fh)
  .intersect(cylinder(fh, r0 + fh * Math.tan(rad(60)), { top: r0 }))
  .translate(0, 0, fl);
const body = union(
  externalThread({ d, pitch, z0: -tl, z1: fl + 0.01, tip: "bottom" }),
  flange,
  cylinder(zn - z1 + 0.01, rn).translate(0, 0, z1 - 0.01),
).subtract(cylinder(tl + zn + 2, bore).translate(0, 0, -tl - 1));
const flangeSeal = polygon([[d / 2, 0], [0.47 * af, 0], [0.47 * af, fl], [d / 2, fl]]).revolve(360);

// Cap nut: the hex, rising into a cone to the cable opening; hollow over
// the neck and round the seal, a lip holding the seal down.
const capH = H - z2;
const corner = af / Math.sqrt(3);
const capOuter = hexPrism(af, capH).intersect(polygon([[0, 0], [corner, 0], [corner, 0.4 * capH], [0.47 * af, 0.6 * capH], [rTop, capH], [0, capH]]).revolve(360)).translate(0, 0, z2);
const lip = 1.2;
const cap = capOuter
  .subtract(cylinder(zn - z2 + 0.01, rn).translate(0, 0, z2 - 0.01))
  .subtract(cylinder(H - lip - zn + 0.01, rs).translate(0, 0, zn - 0.01))
  .subtract(cylinder(H, bore + 0.3).translate(0, 0, zn));
const seal = polygon([[bore, zn], [rs, zn], [rs, H - lip], [bore, H - lip]]).revolve(360);

// Locknut under the panel.
const nut = hexPrism(af, nh)
  .intersect(cylinder(nh, r0 + nh * Math.tan(rad(60)), { top: r0 }))
  .translate(0, 0, -panel - nh)
  .subtract(internalThreadTool({ d, pitch, z0: -panel - nh, z1: -panel }));

const massG = 1.14e-3 * (body.volume + cap.volume + nut.volume) + 1.25e-3 * (seal.volume + flangeSeal.volume);
stock({
  partNumber: `Cable gland ${name} PA66 ${num(cMin)}-${num(cMax)}`,
  description: `Nylon cable gland ${name}, IP68, for cable Ø${num(cMin)}–${num(cMax)} mm, ${num(af)} mm hex, with locknut: panel hole Ø${num(hole)}`,
  unitPrice: lookup(PRICE, p.size, "price"),
  currency: "USD",
  pack: 10,
  massG: Math.round(massG * 100) / 100,
  sources: [
    supplierSearch("McMaster-Carr", "cord grips"),
    supplierSearch("Misumi", `cable gland ${name} nylon`),
    supplierSearch("Amazon", `${name} nylon cable gland IP68`),
    supplierSearch("AliExpress", `${name} cable gland nylon IP68 ${num(cMin)}-${num(cMax)}mm`),
  ],
});
metric("Panel hole Ø", hole, "mm");
metric("Cable Ø min", cMin, "mm");
metric("Cable Ø max", cMax, "mm");
metric("Across flats", af, "mm");
metric("Thread length", tl, "mm");
metric("Height", H, "mm");

// The panel's outer face it seals on, the cable's axis (from the entry up),
// where the cable goes in.
connector("Panel face", { at: [0, 0, 0], axis: [0, 0, -1], part: "Flange seal" });
connector("Axis", { at: [0, 0, -tl], axis: [0, 0, 1], part: "Body" });
connector("Cable entry", { at: [0, 0, H], axis: [0, 0, 1], part: "Cap nut" });
return [
  part("Body", body, { material: "Nylon PA66" }),
  part("Cap nut", cap, { material: "Nylon PA66" }),
  part("Seal", seal, { material: "NBR rubber" }),
  part("Flange seal", flangeSeal, { material: "NBR rubber" }),
  part("Locknut", nut, { material: "Nylon PA66" }),
];
