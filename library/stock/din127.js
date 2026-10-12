// Spring lock washer, square ends, DIN 127 form B, stainless A2.
//
// Source: DIN 127:1987 table 1 (form B) — inside diameter d1 (min), outside
// diameter d2 (max), section thickness s; free height h ≈ 2 s (the split
// ends set apart by one thickness). The section's width is (d2 − d1)/2.
//
// One turn of a rectangular section rising s over the turn, the split at
// +X, the low end resting on z = 0; the free height reaches z = 2 s. The
// helix is computed as a mesh (faceted in STEP), as the true threads are.
// Mass: this nominal geometry at 7.9 g/cm³ (stainless).

const DIN127 = {
  //     d1    d2    s
  2: [2.1, 4.4, 0.5],
  2.5: [2.6, 5.1, 0.6],
  3: [3.1, 6.2, 0.8],
  4: [4.1, 7.6, 0.9],
  5: [5.1, 9.2, 1.2],
  6: [6.1, 11.8, 1.6],
  8: [8.2, 14.8, 2.0],
  10: [10.2, 18.1, 2.2],
  12: [12.2, 21.1, 2.5],
};
// Typical price of one washer in a pack of 100 (stainless A2, USD).
const PRICE = { 2: 0.02, 2.5: 0.02, 3: 0.015, 4: 0.018, 5: 0.022, 6: 0.028, 8: 0.045, 10: 0.07, 12: 0.1 };

const d = p.d;
const [d1, d2, s] = lookup(DIN127, d, "DIN 127");
const ri = d1 / 2;
const ro = d2 / 2;
const rm = (ri + ro) / 2;
const gap = s / rm; // the split, one thickness wide at the mean radius (radians)
const a0 = gap / 2;
const a1 = 2 * Math.PI - gap / 2;
const rise = s;

// The section swept round the turn: inner-bottom, outer-bottom, outer-top,
// inner-top at each step, the square-cut ends capped.
const steps = Math.max(48, segmentsFor(ro));
const positions = [];
for (let i = 0; i <= steps; i++) {
  const u = i / steps;
  const a = a0 + (a1 - a0) * u;
  const z = rise * u;
  const [c, sn] = [Math.cos(a), Math.sin(a)];
  for (const [r, zz] of [[ri, z], [ro, z], [ro, z + s], [ri, z + s]]) positions.push([r * c, r * sn, zz]);
}
const at = (i, j) => i * 4 + j;
const triangles = [];
for (let i = 0; i < steps; i++) {
  for (let j = 0; j < 4; j++) {
    const k = (j + 1) % 4;
    triangles.push([at(i, j), at(i + 1, j), at(i + 1, k)], [at(i, j), at(i + 1, k), at(i, k)]);
  }
}
triangles.push([at(0, 0), at(0, 1), at(0, 2)], [at(0, 0), at(0, 2), at(0, 3)]);
triangles.push([at(steps, 0), at(steps, 2), at(steps, 1)], [at(steps, 0), at(steps, 3), at(steps, 2)]);
const washer = fromMesh(positions, triangles);

const massG = 7.9e-3 * washer.volume;
stock({
  standard: "DIN 127",
  partNumber: `DIN 127 B ${num(d)} A2`,
  description: `Spring lock washer for M${num(d)}, form B (square ends), ${num(d1)} × ${num(d2)} × ${num(s)} mm, stainless A2`,
  unitPrice: lookup(PRICE, d, "price"),
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 10000) / 10000,
  sources: [
    supplierSearch("McMaster-Carr", "split lock washers"),
    supplierSearch("Bolt Depot", `M${num(d)} split lock washer stainless`),
    supplierSearch("Misumi", `spring washer M${num(d)} stainless`),
    supplierSearch("AliExpress", `M${num(d)} spring lock washer DIN 127 A2 stainless`),
  ],
});
metric("Inside diameter", d1, "mm");
metric("Outside diameter", d2, "mm");
metric("Thickness", s, "mm");
metric("Free height", s + rise, "mm");

// Flattened when the screw is tight: the faces then sit at 0 and s.
connector("Bottom face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top face", { at: [0, 0, s], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
return washer;
