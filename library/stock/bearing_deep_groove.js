// Deep groove ball bearing, sealed both sides (-2RS), single row.
//
// Sources: boundary dimensions d × D × B from ISO 15:2017 (dimension series
// 10 for 60xx, 18 for 67xx/68xx (618), 19 for 69xx (619), 02/03-derived
// miniature 62x/608/688 as listed by SKF, NSK and NMB catalogues). Ball
// count and ball size (inch-series balls, ISO 3290) are typical values for
// the series (SKF/NSK internal-design tables vary slightly by maker). 62xx
// (dimension series 02) and 603–609 boundary dimensions from ISO 15 / SKF;
// MR63–MR128 from the MinebeaMitsumi (NMB) miniature catalogue. Masses
// are catalogue values for steel sealed bearings (SKF, NSK, Fersa: 608 12 g,
// 6000 19 g, 6704 5 g, 6200 32 g, 6204 106 g, MR105 1.1 g …); the 67xx
// ultra-thin series masses not published by every maker are estimated from
// the 6704 figure by section area.
//
// Stands with its axis along +Z, face A on z = 0 and face B on z = B. The
// rings, balls, cage and seals are drawn by radialBallBearing (bearing.js).

//          d   D   B  balls   Dw     mass g  USD
const DGBB = {
  "3x10x4": ["623", 7, 1.5875, 1.6, 0.6],
  "4x13x5": ["624", 7, 2.381, 3.1, 0.6],
  "5x16x5": ["625", 7, 2.778, 4.9, 0.6],
  "6x19x6": ["626", 7, 3.175, 8.2, 0.7],
  "8x22x7": ["608", 7, 3.969, 12, 0.45],
  "8x16x5": ["688", 10, 2.381, 3.6, 0.9],
  "10x26x8": ["6000", 7, 4.762, 19, 1.1],
  "12x28x8": ["6001", 8, 4.762, 22, 1.2],
  "15x32x9": ["6002", 9, 4.762, 30, 1.4],
  "17x35x10": ["6003", 10, 4.762, 39, 1.6],
  "20x42x12": ["6004", 9, 6.35, 69, 2.2],
  "25x47x12": ["6005", 10, 6.35, 80, 2.6],
  "10x15x4": ["6700", 14, 1.5875, 1.9, 1.6],
  "12x18x4": ["6701", 14, 2.0, 2.7, 1.7],
  "15x21x4": ["6702", 17, 2.0, 3.3, 1.8],
  "17x23x4": ["6703", 18, 2.0, 3.6, 1.9],
  "20x27x4": ["6704", 18, 2.381, 5.0, 2.1],
  "25x32x4": ["6705", 21, 2.381, 6.1, 2.4],
  "10x19x5": ["6800", 12, 2.381, 5.5, 1.3],
  "12x21x5": ["6801", 14, 2.381, 6.3, 1.4],
  "15x24x5": ["6802", 16, 2.381, 7.5, 1.5],
  "17x26x5": ["6803", 18, 2.381, 8.3, 1.6],
  "20x32x7": ["6804", 18, 3.175, 19, 2.2],
  "25x37x7": ["6805", 20, 3.175, 22, 2.6],
  "30x42x7": ["6806", 24, 3.175, 26, 3.0],
  "35x47x7": ["6807", 27, 3.175, 29, 3.4],
  "10x22x6": ["6900", 10, 3.175, 9, 1.4],
  "12x24x6": ["6901", 11, 3.175, 11, 1.5],
  "15x28x7": ["6902", 11, 3.969, 16, 1.7],
  "17x30x7": ["6903", 12, 3.969, 18, 1.9],
  "20x37x9": ["6904", 12, 4.762, 37, 2.5],
  "25x42x9": ["6905", 14, 4.762, 42, 2.9],
  // 60x miniature (dimension series 10, extended) and 62xx (series 02).
  "3x9x5": ["603", 6, 1.5875, 1.5, 0.7],
  "5x14x5": ["605", 7, 2.381, 3.5, 0.7],
  "7x19x6": ["607", 7, 3.175, 8, 0.7],
  "9x24x7": ["609", 7, 3.969, 15, 0.9],
  "10x30x9": ["6200", 8, 4.762, 32, 1.3],
  "12x32x10": ["6201", 7, 5.953, 37, 1.5],
  "15x35x11": ["6202", 8, 5.953, 45, 1.7],
  "17x40x12": ["6203", 8, 6.747, 65, 2.0],
  "20x47x14": ["6204", 8, 7.938, 106, 2.6],
  "25x52x15": ["6205", 9, 7.938, 128, 3.0],
  // MR miniature series (ISO 15 dimension series 18/19 below 10 mm bore).
  "3x6x2.5": ["MR63", 8, 0.794, 0.27, 0.9],
  "5x8x2.5": ["MR85", 11, 0.794, 0.4, 0.9],
  "5x10x4": ["MR105", 8, 1.5875, 1.1, 0.8],
  "5x11x4": ["MR115", 8, 1.5875, 1.5, 0.9],
  "8x12x3.5": ["MR128", 12, 1.0, 1.1, 1.0],
};

const d = p.d;
const D = p.D;
const B = p.B;
const key = `${num(d)}x${num(D)}x${num(B)}`;
const [series, Z, Dw, catalogueMass, price] = lookup(DGBB, key, "deep groove ball bearing");
const name = `${series}-2RS`;

const { inner, outer, balls, cage, closures } = radialBallBearing({ d, D, B, Z, Dw, closure: "2RS" });

stock({
  standard: "ISO 15",
  partNumber: name,
  description: `Deep groove ball bearing ${name}, ${num(d)}×${num(D)}×${num(B)} mm, rubber sealed both sides`,
  mpn: name,
  unitPrice: price,
  currency: "USD",
  pack: 10,
  massG: catalogueMass,
  sources: [
    supplierSearch("VXB", `${name} bearing`),
    supplierSearch("McMaster-Carr", "ball bearings"),
    supplierSearch("Misumi", `${series} bearing`),
    supplierSearch("AliExpress", `${name} bearing ${num(d)}x${num(D)}x${num(B)}`),
    supplierSearch("Amazon", `${name} ball bearing`),
  ],
});
metric("Bore", d, "mm");
metric("Outside diameter", D, "mm");
metric("Width", B, "mm");
metric("Balls", Z);

bearingConnectors(D, B);

return [
  part("Inner ring", inner, { material: "Bearing steel 52100" }),
  part("Outer ring", outer, { material: "Bearing steel 52100" }),
  part("Balls", balls, { material: "Bearing steel 52100" }),
  part("Cage", cage, { material: "Steel (cage)" }),
  part("Seals", closures, { material: "Nitrile rubber (NBR)" }),
];
