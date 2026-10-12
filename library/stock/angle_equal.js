// Hot-rolled equal-leg angles, EN 10056-1, cut to length.
//
// Sources: EN 10056-1:2017 tables (as Tata Steel's and ArcelorMittal's
// merchant-bar catalogues print them): legs a × b, thickness t, root radius
// r1, toe radius r2, and G, the mass per metre at 7850 kg/m³. The modelled
// section is within 0.6 % of G. S235JR to EN 10025-2, sold by the metre and
// sawn to length.
//
// Stands on its end: the heel on the origin, leg a up +Y, leg b along +X,
// the length along +Z from z = 0. The axis connector is at the centroid.

const TABLE = { // r1, r2 (mm), G (kg/m)
  "20x3": [3.5, 2, 0.882],
  "25x3": [3.5, 2, 1.12],
  "25x4": [3.5, 2, 1.45],
  "30x3": [5, 2.5, 1.36],
  "30x4": [5, 2.5, 1.78],
  "35x4": [5, 2.5, 2.09],
  "40x4": [6, 3, 2.42],
  "40x5": [6, 3, 2.97],
  "45x4.5": [7, 3.5, 3.06],
  "50x4": [7, 3.5, 3.06],
  "50x5": [7, 3.5, 3.77],
  "50x6": [7, 3.5, 4.47],
  "60x5": [8, 4, 4.57],
  "60x6": [8, 4, 5.42],
  "60x8": [8, 4, 7.09],
  "65x7": [9, 4.5, 6.83],
  "70x6": [9, 4.5, 6.38],
  "70x7": [9, 4.5, 7.38],
  "75x6": [9, 4.5, 6.85],
  "75x8": [9, 4.5, 8.99],
  "80x8": [10, 5, 9.63],
  "80x10": [10, 5, 11.9],
  "90x7": [11, 5.5, 9.61],
  "90x8": [11, 5.5, 10.9],
  "90x9": [11, 5.5, 12.2],
  "90x10": [11, 5.5, 13.4],
  "100x8": [12, 6, 12.2],
  "100x10": [12, 6, 15],
  "100x12": [12, 6, 17.8],
  "120x10": [13, 6.5, 18.2],
  "120x12": [13, 6.5, 21.6],
  "150x10": [16, 8, 23],
  "150x12": [16, 8, 27.3],
  "150x15": [16, 8, 33.8],
  "160x15": [17, 8.5, 36.2],
  "180x16": [18, 9, 43.5],
  "180x18": [18, 9, 48.6],
  "200x16": [18, 9, 48.5],
  "200x20": [18, 9, 59.9],
  "200x24": [18, 9, 71.1],
};

const a = p.a;
const b = p.a;
const t = p.t;
const [r1, r2, G] = lookup(TABLE, `${num(p.a)}x${num(p.t)}`, "EN 10056-1 angle");
const L = p.length;
const { solid } = angleSection({ a, b, t, r1, r2 }, L);
const designation = `L ${num(a)}×${num(a)}×${num(t)}`;
cutToLength({
  designation,
  grade: "S235JR",
  standard: "EN 10056-1",
  description: `${designation} equal angle, hot rolled EN 10056-1`,
  kgPerM: G,
  rate: "bar",
  L,
  sources: steelSources(`angle ${num(a)}x${num(b)}x${num(t)} steel`),
});
metric("Root radius r1", r1, "mm");
metric("Toe radius r2", r2, "mm");
return solid;
