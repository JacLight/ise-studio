// Hot-rolled unequal-leg angles, EN 10056-1, cut to length.
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
  "30x20x3": [4, 2, 1.12],
  "40x20x4": [4, 2, 1.77],
  "40x25x4": [4, 2, 1.93],
  "45x30x4": [4.5, 2.25, 2.25],
  "50x30x5": [5, 2.5, 2.96],
  "60x30x5": [5, 2.5, 3.37],
  "60x40x5": [6, 3, 3.76],
  "60x40x6": [6, 3, 4.46],
  "65x50x5": [6, 3, 4.35],
  "70x50x6": [7, 3.5, 5.41],
  "75x50x6": [7, 3.5, 5.65],
  "75x50x8": [7, 3.5, 7.39],
  "80x40x6": [7, 3.5, 5.41],
  "80x40x8": [7, 3.5, 7.07],
  "80x60x7": [8, 4, 7.36],
  "100x50x6": [8, 4, 6.84],
  "100x50x8": [8, 4, 8.97],
  "100x65x7": [10, 5, 8.77],
  "100x75x8": [10, 5, 10.6],
  "100x75x10": [10, 5, 13],
  "120x80x8": [11, 5.5, 12.2],
  "120x80x10": [11, 5.5, 15],
  "125x75x8": [11, 5.5, 12.2],
  "150x75x10": [11, 5.5, 17],
  "150x90x10": [12, 6, 18.2],
  "150x100x10": [12, 6, 19],
  "200x100x10": [15, 7.5, 23],
  "200x100x12": [15, 7.5, 27.3],
};

const a = p.a;
const b = p.b;
const t = p.t;
const [r1, r2, G] = lookup(TABLE, `${num(p.a)}x${num(p.b)}x${num(p.t)}`, "EN 10056-1 angle");
const L = p.length;
const { solid } = angleSection({ a, b, t, r1, r2 }, L);
const designation = `L ${num(a)}×${num(b)}×${num(t)}`;
cutToLength({
  designation,
  grade: "S235JR",
  standard: "EN 10056-1",
  description: `${designation} unequal angle, hot rolled EN 10056-1`,
  kgPerM: G,
  rate: "bar",
  L,
  sources: steelSources(`angle ${num(a)}x${num(b)}x${num(t)} steel`),
});
metric("Root radius r1", r1, "mm");
metric("Toe radius r2", r2, "mm");
return solid;
