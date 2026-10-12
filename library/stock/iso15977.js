// Open end blind rivet with break pull mandrel and protruding (dome) head,
// ISO 15977 (DIN 7337 A): aluminium alloy AlMg2.5 body, steel mandrel.
//
// Source: ISO 15977:2002 table 1 — body diameter d, head diameter dk (max),
// head height k (max), mandrel diameter dm (max), mandrel protrusion
// p ≥ 25 mm; the recommended hole (d + 0.1) and the grip ranges for each
// body length l from the makers' tables (Gesipa, POP: the same within
// 0.5 mm). The mandrel's head sits just past the body's tail, about 0.9 d
// across, as the makers draw it.
//
// Unset, as bought: the head's bearing face on z = 0, the body down −Z to
// z = −l, the mandrel up +Z out of the head (where the tool grips it).
// Mass: this nominal geometry, body at 2.68 g/cm³, mandrel at 7.85.

const ISO15977 = {
  //    dk    k    dm   hole
  3.2: [6.7, 1.3, 2.0, 3.3],
  4: [8.4, 1.7, 2.45, 4.1],
  4.8: [10.1, 2.0, 2.95, 4.9],
};
// Grip ranges (min, max) by "d×l".
const GRIP = {
  "3.2×6": [1.5, 3.5], "3.2×8": [3.5, 5.0], "3.2×10": [5.0, 6.5], "3.2×12": [6.5, 8.5],
  "4×6": [1.5, 3.0], "4×8": [3.0, 4.5], "4×10": [4.5, 6.5], "4×12": [6.5, 8.5], "4×16": [10.0, 12.5],
  "4.8×8": [2.5, 4.5], "4.8×10": [4.5, 6.0], "4.8×12": [6.0, 8.0], "4.8×16": [9.5, 12.0], "4.8×20": [13.0, 16.0],
};
// Typical price of one rivet in a box of 500 (Al/steel, USD): base + per mm.
const PRICE = { 3.2: [0.012, 0.001], 4: [0.016, 0.0012], 4.8: [0.022, 0.0015] };

const d = p.d;
const l = p.length;
const [dk, k, dm, hole] = lookup(ISO15977, d, "ISO 15977");
const [gripMin, gripMax] = lookup(GRIP, `${num(d)}×${num(l)}`, "ISO 15977 grip");
const protrude = 26; // mandrel above the head

// Body: a dome head (a spherical cap from a short edge, like ISO 7380's)
// over the tube, the bore dm right through.
const edge = 0.25 * k;
const rise = k - edge;
const R = ((dk / 2) ** 2 + rise * rise) / (2 * rise);
const head = cylinder(k, dk / 2).intersect(sphere(R).translate(0, 0, k - R));
const tail = 0.1 * d; // the tail's outer edge broken
const tube = polygon([[0, -l], [d / 2 - tail, -l], [d / 2, -l + tail], [d / 2, 0.01], [0, 0.01]]).revolve(360);
const body = head.add(tube).subtract(cylinder(l + k + 2, dm / 2).translate(0, 0, -l - 1));
// Mandrel: the stem through the bore and out of the head, a 45° point at
// its top, its head under the tail.
const mh = 0.35 * d;
const md = 0.9 * d;
const stemTop = k + protrude;
const mandrel = union(
  polygon([[0, -l], [dm / 2, -l], [dm / 2, stemTop - 0.3 * dm], [0.2 * dm, stemTop], [0, stemTop]]).revolve(360),
  polygon([[0, -l - mh], [md / 2 - 0.1 * d, -l - mh], [md / 2, -l - mh + 0.1 * d], [md / 2, -l], [0, -l]]).revolve(360),
);

const massG = 2.68e-3 * body.volume + 7.85e-3 * mandrel.volume;
const [base, perMm] = lookup(PRICE, d, "price");
const size = `${num(d)}×${num(l)}`;
stock({
  standard: "ISO 15977",
  partNumber: `ISO 15977 ${size} AlA/St`,
  description: `Blind rivet Ø${num(d)} × ${num(l)}, open end, dome head Ø${num(dk)}, aluminium / steel mandrel: grip ${num(gripMin)}–${num(gripMax)} mm, drill Ø${num(hole)}`,
  unitPrice: Math.round((base + perMm * l) * 1000) / 1000,
  currency: "USD",
  pack: 500,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "blind rivets"),
    supplierSearch("Misumi", `blind rivet aluminium ${num(d)} x ${num(l)}`),
    supplierSearch("Bolt Depot", `${num(d)}mm aluminum blind rivet`),
    supplierSearch("AliExpress", `${num(d)}x${num(l)} blind rivet aluminum DIN 7337`),
  ],
});
metric("Hole Ø", hole, "mm");
metric("Grip min", gripMin, "mm");
metric("Grip max", gripMax, "mm");
metric("Head Ø", dk, "mm");

// The head's bearing face, the axis from the tail up, and the mandrel's
// end where the riveting tool takes it.
connector("Head seat", { at: [0, 0, 0], axis: [0, 0, -1], part: "Body" });
connector("Axis", { at: [0, 0, -l], axis: [0, 0, 1], part: "Body" });
connector("Mandrel", { at: [0, 0, stemTop], axis: [0, 0, 1], part: "Mandrel" });
return [part("Body", body, { material: "Aluminium AlMg2.5" }), part("Mandrel", mandrel, { material: "Steel (mandrel)" })];
