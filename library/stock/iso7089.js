// Plain washer, normal series, product grade A, ISO 7089 (DIN 125-A sizes),
// stainless A2.
//
// Source: ISO 7089:2000 table 1 — inside diameter d1 (min, nominal),
// outside diameter d2 (max, nominal), thickness h (nominal).
//
// Lies on its bottom face at z = 0, top face at z = h, axis along +Z.
// Mass: the nominal ring at 7.9 g/cm³ (stainless).

const ISO7089 = {
  //    d1    d2    h
  2: [2.2, 5.0, 0.3],
  2.5: [2.7, 6.0, 0.5],
  3: [3.2, 7.0, 0.5],
  4: [4.3, 9.0, 0.8],
  5: [5.3, 10.0, 1.0],
  6: [6.4, 12.0, 1.6],
  8: [8.4, 16.0, 1.6],
};
// Typical price of one washer in a pack of 100 (stainless A2, USD).
const PRICE = { 2: 0.015, 2.5: 0.015, 3: 0.012, 4: 0.015, 5: 0.018, 6: 0.022, 8: 0.035 };

const d = p.d;
const [d1, d2, h] = lookup(ISO7089, d, "ISO 7089");
const washer = polygon([[d1 / 2, 0], [d2 / 2, 0], [d2 / 2, h], [d1 / 2, h]]).revolve(360);

const massG = 7.9e-3 * (Math.PI / 4) * (d2 * d2 - d1 * d1) * h;
stock({
  standard: "ISO 7089",
  partNumber: `ISO 7089 ${num(d)} A2`,
  description: `Flat washer for M${num(d)}, ${num(d1)} × ${num(d2)} × ${num(h)} mm, stainless A2`,
  unitPrice: lookup(PRICE, d, "price"),
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 10000) / 10000,
  sources: [
    supplierSearch("McMaster-Carr", "washers"),
    supplierSearch("Bolt Depot", `M${num(d)} flat washer stainless DIN 125`),
    supplierSearch("Misumi", `flat washer M${num(d)} stainless`),
    supplierSearch("AliExpress", `M${num(d)} flat washer DIN 125 A2 stainless`),
  ],
});
metric("Inside diameter", d1, "mm");
metric("Outside diameter", d2, "mm");
metric("Thickness", h, "mm");

connector("Bottom face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top face", { at: [0, 0, h], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
return washer;
