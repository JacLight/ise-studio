// D-cut shaft: precision-ground stainless shaft (h7) with one flat its
// whole length, so set screws and D-bore hubs (pulleys, gears, wheels,
// couplings) grip it without slipping; chamfered ends.
//
// Sources: Misumi SFJD / PSFJD one-flat shafts and the metric D-shafts sold
// for robotics (ServoCity / goBILDA pattern): flat depth 0.5 mm on Ø5 and
// Ø6 (4.5 / 5.5 across the flat), 0.75 mm on Ø8 (7.25), end chamfers C0.3
// (C0.5 on Ø8); 303 stainless, 8.0 g/cm³. Mass: the modelled section.
//
// Axis along +Z, end A on z = 0, end B on z = L; the flat faces +X.

const d = p.d;
const L = p.length;
const FLAT = { 5: [0.5, 0.3, 0.012], 6: [0.5, 0.3, 0.015], 8: [0.75, 0.5, 0.02] }; // depth, chamfer, USD per mm
const [depth, c, perMm] = lookup(FLAT, d, "D-cut shaft");
const across = d - depth;
const rod = polygon([[0, 0], [d / 2 - c, 0], [d / 2, c], [d / 2, L - c], [d / 2 - c, L], [0, L]]).revolve(360);
// The flat: everything beyond x = d/2 − depth.
const cut = box(d, d + 2, L + 2).translate(d / 2 - depth, -d / 2 - 1, -1);
const shaft = rod.subtract(cut);

const segment = (r, h) => r * r * Math.acos((r - h) / r) - (r - h) * Math.sqrt(2 * r * h - h * h);
const massG = (Math.PI * (d / 2) ** 2 - segment(d / 2, depth)) * L * 8.0e-3;
const size = `Ø${num(d)}×${num(L)}`;
stock({
  partNumber: `D-shaft ${size} flat ${num(across)}`,
  description: `D-cut shaft ${size} mm, one flat full length (${num(across)} mm across), 303 stainless, h7`,
  unitPrice: cents(1.2 + perMm * L),
  currency: "USD",
  pack: 1,
  massG: Math.round(massG * 10) / 10,
  sources: [
    { supplier: "Misumi", sku: `SFJD${num(d)}-${num(L)}`, url: `https://us.misumi-ec.com/vona2/result/?Keyword=SFJD${num(d)}-${num(L)}` },
    supplierSearch("McMaster-Carr", "d-profile shafts"),
    supplierSearch("AliExpress", `${num(d)}mm D shaft ${num(L)}mm stainless`),
    supplierSearch("Amazon", `${num(d)}mm D shaft ${num(L)}mm`),
  ],
});
metric("Diameter", d, "mm");
metric("Across flat", across, "mm");
metric("Length", L, "mm");

connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("End A", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("End B", { at: [0, 0, L], axis: [0, 0, 1] });
connector("Flat", { at: [d / 2 - depth, 0, L / 2], axis: [1, 0, 0], x: [0, 0, 1] });
return shaft;
