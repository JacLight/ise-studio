// Deep groove ball bearing, sealed both sides (-2RS), single row.
//
// Sources: boundary dimensions d × D × B from ISO 15:2017 (dimension series
// 10 for 60xx, 18 for 67xx/68xx (618), 19 for 69xx (619), 02/03-derived
// miniature 62x/608/688 as listed by SKF, NSK and NMB catalogues). Ball
// count and ball size (inch-series balls, ISO 3290) are typical values for
// the series (SKF/NSK internal-design tables vary slightly by maker). Masses
// are catalogue values for steel sealed bearings (SKF, NSK, Fersa: 608 12 g,
// 6000 19 g, 6704 5 g …); the 67xx ultra-thin series masses not published
// by every maker are estimated from the 6704 figure by section area.
//
// Stands with its axis along +Z, face A on z = 0 and face B on z = B.
// Raceways are true circular grooves (radius 0.52·Dw, the usual
// osculation), lands at dm ∓ 0.66·Dw, seals sit 0.15 mm in from each face
// in a recess of the outer ring, and a pressed-steel ribbon cage holds
// the balls apart.

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
};

const d = p.d;
const D = p.D;
const B = p.B;
const key = `${num(d)}x${num(D)}x${num(B)}`;
const [series, Z, Dw, catalogueMass, price] = lookup(DGBB, key, "deep groove ball bearing");
const name = `${series}-2RS`;

const dm = (d + D) / 2; // pitch diameter
const rp = dm / 2;
const rg = 0.52 * Dw; // groove radius (osculation 52 %)
const ri = rp - 0.33 * Dw; // inner ring land
const ro = rp + 0.33 * Dw; // outer ring land
const zg = Math.sqrt(rg * rg - (0.33 * Dw) ** 2); // where the groove meets the land
const c = Math.min(0.3, 0.06 * (D - d)); // corner chamfers
const ts = Math.min(1, Math.max(0.3, 0.09 * B)); // seal thickness
const rs = ro + 0.4 * (D / 2 - ro); // seal recess in the outer ring
const zs = 0.15 + ts; // seal recess depth from each face

/** Groove arc points about the ball centre, on the inner (-1) or outer (+1) side, from z low to high. */
function groove(side) {
  const a = Math.asin(zg / rg);
  const n = Math.max(6, Math.ceil(segmentsFor(rg) * (2 * a) / (2 * Math.PI)));
  const pts = [];
  for (let i = 0; i <= n; i++) {
    const t = -a + (2 * a * i) / n;
    pts.push([rp + side * rg * Math.cos(t), B / 2 + rg * Math.sin(t)]);
  }
  return pts;
}

const inner = polygon([
  [d / 2 + c, 0],
  [ri, 0],
  ...groove(-1),
  [ri, B],
  [d / 2 + c, B],
  [d / 2, B - c],
  [d / 2, c],
]).revolve(360);

const outer = polygon([
  [rs, 0],
  [D / 2 - c, 0],
  [D / 2, c],
  [D / 2, B - c],
  [D / 2 - c, B],
  [rs, B],
  [rs, B - zs],
  [ro, B - zs],
  ...groove(1).reverse(),
  [ro, zs],
  [rs, zs],
]).revolve(360);

const balls = [];
for (let i = 0; i < Z; i++) {
  const a = (2 * Math.PI * i) / Z;
  balls.push(sphere(Dw / 2).translate(rp * Math.cos(a), rp * Math.sin(a), B / 2));
}

// Seals: rubber-faced discs from just over the inner land out into the recess.
const seal = polygon([[ri + 0.08, 0], [rs, 0], [rs, ts], [ri + 0.08, ts]]).revolve(360);
const seals = seal.translate(0, 0, 0.15).add(seal.translate(0, 0, B - zs));

// Cage: a pressed-steel ribbon band round the pitch circle with a pocket per ball.
const band = polygon([[rp - 0.18 * Dw, B / 2 - 0.35 * Dw], [rp + 0.18 * Dw, B / 2 - 0.35 * Dw], [rp + 0.18 * Dw, B / 2 + 0.35 * Dw], [rp - 0.18 * Dw, B / 2 + 0.35 * Dw]]).revolve(360);
const pockets = [];
for (let i = 0; i < Z; i++) {
  const a = (2 * Math.PI * i) / Z;
  pockets.push(sphere(Dw / 2 + 0.03).translate(rp * Math.cos(a), rp * Math.sin(a), B / 2));
}
const cage = band.subtract(union(pockets));

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

connector("Bore", { at: [0, 0, B / 2], axis: [0, 0, 1] });
connector("Outer seat", { at: [D / 2, 0, B / 2], axis: [1, 0, 0], x: [0, 0, 1] });
connector("Face A", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Face B", { at: [0, 0, B], axis: [0, 0, 1] });

return [
  part("Inner ring", inner, { material: "Bearing steel 52100" }),
  part("Outer ring", outer, { material: "Bearing steel 52100" }),
  part("Balls", union(balls), { material: "Bearing steel 52100" }),
  part("Cage", cage, { material: "Steel (cage)" }),
  part("Seals", seals, { material: "Nitrile rubber (NBR)" }),
];
