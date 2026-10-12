// Plain washer, large series (fender / penny washer), product grade A,
// ISO 7093-1 (DIN 9021 sizes: outside ≈ 3 d), stainless A2.
//
// Source: ISO 7093-1:2000 table 1 — inside diameter d1 (min, nominal),
// outside diameter d2 (max, nominal), thickness h (nominal).
//
// Lies on its bottom face at z = 0, top face at z = h, axis along +Z.
// Mass: the nominal ring at 7.9 g/cm³ (stainless).

const ISO7093 = {
  //     d1    d2    h
  3: [3.2, 9.0, 0.8],
  4: [4.3, 12.0, 1.0],
  5: [5.3, 15.0, 1.2],
  6: [6.4, 18.0, 1.6],
  8: [8.4, 24.0, 2.0],
  10: [10.5, 30.0, 2.5],
  12: [13.0, 37.0, 3.0],
};
// Typical price of one washer in a pack of 100 (stainless A2, USD).
const PRICE = { 3: 0.02, 4: 0.025, 5: 0.03, 6: 0.04, 8: 0.07, 10: 0.11, 12: 0.17 };

const d = p.d;
const [d1, d2, h] = lookup(ISO7093, d, "ISO 7093-1");
const washer = polygon([[d1 / 2, 0], [d2 / 2, 0], [d2 / 2, h], [d1 / 2, h]]).revolve(360);

const massG = 7.9e-3 * (Math.PI / 4) * (d2 * d2 - d1 * d1) * h;
stock({
  standard: "ISO 7093-1",
  partNumber: `ISO 7093-1 ${num(d)} A2`,
  description: `Large flat washer (fender) for M${num(d)}, ${num(d1)} × ${num(d2)} × ${num(h)} mm, stainless A2`,
  unitPrice: lookup(PRICE, d, "price"),
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 10000) / 10000,
  sources: [
    supplierSearch("McMaster-Carr", "oversized washers"),
    supplierSearch("Bolt Depot", `M${num(d)} fender washer stainless DIN 9021`),
    supplierSearch("Misumi", `large washer M${num(d)} stainless`),
    supplierSearch("AliExpress", `M${num(d)} large flat washer DIN 9021 A2 stainless`),
  ],
});
metric("Inside diameter", d1, "mm");
metric("Outside diameter", d2, "mm");
metric("Thickness", h, "mm");

connector("Bottom face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top face", { at: [0, 0, h], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
return washer;
