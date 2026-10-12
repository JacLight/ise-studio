// Hexagon head bolt / screw, product grade A, stainless A2-70: the shape
// ISO 4017 (threaded to the head) and ISO 4014 (plain shank under the head)
// share. The standard's own file runs first and sets STANDARD and FULL.
//
// Source: ISO 4014:2011 / ISO 4017:2011 table 1 — width across flats s
// (nominal; ISO sizes: M10 16 mm, M12 18 mm where DIN 931/933 had 17/19),
// head height k (nominal), bearing face diameter dw (min), washer face
// height c (between its min and max); thread length b = 2d + 6 (l ≤ 125);
// ISO 261 coarse pitch. The head's top is chamfered 30° from the flats'
// inscribed circle, as cold-forged heads are.
//
// Stands with the head's bearing face (the washer face) on z = 0, head up
// +Z, shank down −Z.
// Mass: this nominal geometry at 7.9 g/cm³ (stainless).

const HEX_HEAD = {
  //     s     k     dw     c
  3: [5.5, 2.0, 4.57, 0.25],
  4: [7.0, 2.8, 5.88, 0.25],
  5: [8.0, 3.5, 6.88, 0.3],
  6: [10.0, 4.0, 8.88, 0.3],
  8: [13.0, 5.3, 11.63, 0.35],
  10: [16.0, 6.4, 14.63, 0.35],
  12: [18.0, 7.5, 16.63, 0.35],
};
// Typical price of one bolt in a pack of 100 (stainless A2, USD): base + per mm.
const BOLT_PRICE = { 3: [0.04, 0.0025], 4: [0.05, 0.003], 5: [0.06, 0.004], 6: [0.08, 0.005], 8: [0.13, 0.008], 10: [0.22, 0.012], 12: [0.32, 0.016] };

const d = p.d;
const L = p.length;
const [s, k, dw, c] = lookup(HEX_HEAD, d, STANDARD);
const pitch = coarsePitch(d);
const b = FULL ? L : Math.min(L, 2 * d + 6);
const plain = L - b; // plain shank under the head (none when fully threaded)

// Head: the washer face, then the hex with its 30° top chamfer (a cone from
// the inscribed circle at the top; the corners are clipped by it).
const r0 = 0.475 * s;
const hex = hexPrism(s, k - c)
  .intersect(cylinder(k - c, r0 + (k - c) * Math.tan(rad(60)), { top: r0 }))
  .translate(0, 0, c);
const face = cylinder(c + 0.01, dw / 2);
const pieces = [hex, face];
if (plain > 0) pieces.push(cylinder(plain + 0.01, d / 2).translate(0, 0, -plain));
pieces.push(externalThread({ d, pitch, z0: -L, z1: -plain + 0.01, real: p.threads >= 0.5, tip: "bottom" }));
const bolt = union(pieces);

const massG = 7.9e-3 * bolt.volume;
const [base, perMm] = lookup(BOLT_PRICE, d, "price");
const size = `M${num(d)}×${num(L)}`;
const kind = FULL ? "hex head screw, threaded to the head" : "hex head bolt, part threaded";
stock({
  standard: STANDARD,
  partNumber: `${STANDARD} ${size} A2-70`,
  description: `${kind[0].toUpperCase()}${kind.slice(1)} ${size}, stainless A2-70, ${num(s)} mm spanner`,
  unitPrice: Math.round((base + perMm * L) * 1000) / 1000,
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "hex head screws"),
    supplierSearch("Bolt Depot", `M${num(d)} x ${num(L)} hex ${FULL ? "cap screw full thread" : "bolt partial thread"} stainless`),
    supplierSearch("Misumi", `hex bolt M${num(d)}x${num(L)} stainless`),
    supplierSearch("AliExpress", `M${num(d)} ${num(L)}mm hex bolt ${FULL ? "DIN 933" : "DIN 931"} A2 stainless`),
  ],
});
metric("Across flats", s, "mm");
metric("Head height", k, "mm");
metric("Thread length", b, "mm");

connector("Head seat", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Axis", { at: [0, 0, -L], axis: [0, 0, 1] });
connector("Head top", { at: [0, 0, k], axis: [0, 0, 1] });
return bolt;
