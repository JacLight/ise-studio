// Single-direction thrust ball bearing: shaft washer, housing washer, ball
// set in a pressed-steel cage. 511xx (ISO 104 dimension series 11) and the
// F…-…M miniature series.
//
// Sources: ISO 104 / SKF thrust ball bearing tables — bore d, outside D,
// height T, housing washer bore D1 and shaft washer outside d1 (51100
// 10×24×9, D1 11; 51101 12×26×9; 51102 15×28×9; 51103 17×30×9; 51104
// 20×35×10; 51105 25×42×11; D1 = d + 1 or 2, d1 = D); miniature F3-8M
// 3×8×3.5, F4-10M 4×10×4, F5-12M 5×12×4, F6-14M 6×14×5, F8-16M 8×16×5,
// F8-19M 8×19×7, F10-18M 10×18×5.5 (common miniature thrust tables,
// e.g. VXB / Kit-8 "F8-16M"), washers d1 = D, D1 = d + 0.2. Ball sizes and
// counts typical for the size (ISO 3290 balls packed round the pitch
// circle). Masses: SKF / FAG catalogue for 511xx (51100 ≈ 0.020 kg …
// 51105 ≈ 0.056 kg); the miniature ones from the steel parts as modelled.
//
// Axis +Z: the shaft washer (bore d, fits the shaft) from z = 0, the housing
// washer (bore D1, clears the shaft; outside D, seats in the housing) up to
// z = T. Both washers have a circular raceway groove (radius 0.54·Dw).

//                    series     D1     Dw    balls  mass g  USD
const THRUST = {
  "10x24x9": ["51100", 11, 4.762, 10, 20, 2.2],
  "12x26x9": ["51101", 13, 4.762, 11, 22, 2.4],
  "15x28x9": ["51102", 16, 4.762, 12, 23, 2.6],
  "17x30x9": ["51103", 18, 4.762, 13, 25, 2.8],
  "20x35x10": ["51104", 21, 5.556, 13, 37, 3.2],
  "25x42x11": ["51105", 26, 6.35, 14, 56, 3.9],
  "3x8x3.5": ["F3-8M", 3.2, 1.5875, 8, 0.85, 0.9],
  "4x10x4": ["F4-10M", 4.2, 2.0, 8, 1.5, 0.9],
  "5x12x4": ["F5-12M", 5.2, 2.381, 8, 2.0, 0.9],
  "6x14x5": ["F6-14M", 6.2, 2.778, 8, 3.4, 1.0],
  "8x16x5": ["F8-16M", 8.2, 2.778, 9, 4.1, 1.0],
  "8x19x7": ["F8-19M", 8.2, 3.175, 10, 9.3, 1.2],
  "10x18x5.5": ["F10-18M", 10.2, 2.778, 11, 5.4, 1.2],
};

const d = p.d;
const D = p.D;
const T = p.B;
const key = `${num(d)}x${num(D)}x${num(T)}`;
const [series, D1, Dw, Z, catalogueMass, price] = lookup(THRUST, key, "thrust ball bearing");
const d1 = D; // shaft washer outside

const rp = (d + D) / 4; // pitch radius
const tw = (T - 0.7 * Dw) / 2; // washer thickness: 0.35·Dw clear between washer and ball centre
const rg = 0.54 * Dw; // groove radius
const hw = Math.sqrt(rg * rg - (0.35 * Dw) ** 2); // groove half width at the face
const c = Math.min(0.3, 0.08 * tw);

/** A washer on z = 0…tw, bore rIn, outside rOut, its raceway groove in the top face. */
function washer(rIn, rOut) {
  const a = Math.asin(hw / rg);
  const n = Math.max(6, Math.ceil((segmentsFor(rg) * 2 * a) / (2 * Math.PI)));
  const arc = [];
  for (let i = 0; i <= n; i++) {
    const t = a - (2 * a * i) / n; // from the outer edge of the groove to the inner
    arc.push([rp + rg * Math.sin(t), T / 2 - rg * Math.cos(t)]);
  }
  return polygon([
    [rIn + c, 0],
    [rOut - c, 0],
    [rOut, c],
    [rOut, tw - c],
    [rOut - c, tw],
    ...arc,
    [rIn + c, tw],
    [rIn, tw - c],
    [rIn, c],
  ]).revolve(360);
}

const shaftWasher = washer(d / 2, d1 / 2);
const housingWasher = washer(D1 / 2, D / 2).mirror(0, 0, 1).translate(0, 0, T);

const balls = [];
const pockets = [];
for (let i = 0; i < Z; i++) {
  const a = (2 * Math.PI * i) / Z;
  balls.push(sphere(Dw / 2).translate(rp * Math.cos(a), rp * Math.sin(a), T / 2));
  pockets.push(sphere(Dw / 2 + 0.03).translate(rp * Math.cos(a), rp * Math.sin(a), T / 2));
}
// Cage: a flat pressed-steel ring round the pitch circle, a pocket per ball.
const cage = polygon([[rp - 0.42 * Dw, T / 2 - 0.18 * Dw], [rp + 0.42 * Dw, T / 2 - 0.18 * Dw], [rp + 0.42 * Dw, T / 2 + 0.18 * Dw], [rp - 0.42 * Dw, T / 2 + 0.18 * Dw]])
  .revolve(360)
  .subtract(union(pockets));

stock({
  standard: series.startsWith("F") ? "Miniature thrust (F…M)" : "ISO 104",
  partNumber: series,
  description: `Thrust ball bearing ${series}, ${num(d)}×${num(D)}×${num(T)} mm, single direction, steel cage`,
  mpn: series,
  unitPrice: price,
  currency: "USD",
  pack: series.startsWith("F") ? 10 : 2,
  massG: catalogueMass,
  sources: [
    supplierSearch("VXB", `${series} thrust bearing`),
    supplierSearch("McMaster-Carr", "thrust ball bearings"),
    supplierSearch("Misumi", `thrust ball bearing ${series}`),
    supplierSearch("AliExpress", `${series} thrust ball bearing ${num(d)}x${num(D)}x${num(T)}`),
    supplierSearch("Amazon", `${series} thrust ball bearing`),
  ],
});
metric("Bore", d, "mm");
metric("Outside diameter", D, "mm");
metric("Height", T, "mm");
metric("Housing washer bore", D1, "mm");
metric("Balls", Z);

bearingConnectors(D, T);

return [
  part("Shaft washer", shaftWasher, { material: "Bearing steel 52100" }),
  part("Housing washer", housingWasher, { material: "Bearing steel 52100" }),
  part("Balls", union(balls), { material: "Bearing steel 52100" }),
  part("Cage", cage, { material: "Steel (cage)" }),
];
