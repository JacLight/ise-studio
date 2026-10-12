// KCD1 snap-in rocker switch (the small 21 × 15 mm boat switch on power
// strips and 3D printers): KCD1-101 SPST ON-OFF, 2 terminals, black or red;
// KCD1-102 SPDT ON-ON, 3 terminals.
//
// Source: KCD1 series drawing (e.g. Würth-style KCD1-101, as sold by LCSC /
// Adafruit #3218): bezel 21 × 15 mm, 2.5 mm proud of the panel; body
// 18.8 × 12.8 mm behind it for a 19.0 × 13.0 mm panel cutout (panels
// 0.75–2 mm, held by two snap tabs); 11.5 mm deep body; rocker 14 × 9 mm,
// rocked 8°; terminals 4.8 × 0.8 mm quick-connect tabs, 7.5 mm long, on
// 5.0 mm centres.
//
// The panel's front face on z = 0 with the bezel above it (+Z), the body
// and terminals below (−Z), the rocker's long side along X.

const MODELS = {
  // [mpn, terminals, rocker colour material, price]
  0: ["KCD1-101", 2, "black", 0.22],
  1: ["KCD1-101", 2, "red", 0.24],
  2: ["KCD1-102", 3, "black", 0.28],
};
const [mpn, poles, colour, price] = MODELS[Math.round(p.model)];
const BW = 21;
const BH = 15;
const B = 2.5; // bezel above the panel
// Bezel: a rounded frame with the rocker's opening.
const bezel = roundedRect(BW, BH, 1.5).extrude(B).subtract(roundedRect(BW - 4, BH - 4, 0.8).extrude(B + 1).translate(0, 0, -0.5));
// Body: through the cutout, the two snap tabs on its long sides (where the
// panel is gripped), down to the terminals' base.
let body = rect(18.8, 12.8, { center: true }).extrude(11.5).translate(0, 0, -11.5)
  .add(rect(BW - 4.2, BH - 4.2, { center: true }).extrude(1).translate(0, 0, -0.01));
for (const sy of [-1, 1]) {
  const tab = prismX([[0, -5.0], [0.7, -1.4], [0, -1.2]], -3, 3);
  body = body.add((sy > 0 ? tab : tab.mirror(0, 1, 0)).translate(0, sy * 6.4, 0));
}
// Rocker: a slab with a dished top, rocked 8° about Y (its pivot across X).
const rocker = box(14, 9, 3).translate(-7, -4.5, 0)
  .subtract(cylinder(10, 16).rotate(90, 0, 0).translate(0, 5, 16 + 1.6))
  .rotate(0, 8, 0).translate(0, 0, B - 1.5);
// Terminals: flat tabs on 5 mm centres along X, with the quick-connect hole.
const tabs = [];
for (let i = 0; i < poles; i++) {
  const x = (i - (poles - 1) / 2) * 5.0;
  tabs.push(box(0.8, 4.8, 7.5 + 0.5).translate(x - 0.4, -2.4, -11.5 - 7.5)
    .subtract(cylinder(1, 0.75).rotate(0, 90, 0).translate(x - 0.5, 0, -11.5 - 7.5 + 4.0)));
}
const { parts, massG } = weighed([
  ["Bezel", bezel, "Polyamide (black)"],
  ["Body", body, "Polyamide (black)"],
  ["Rocker", rocker, colour === "red" ? "Polycarbonate (red)" : "Polyamide (black)"],
  ["Terminals", union(...tabs), "Brass, tin-plated"],
]);
stock({
  partNumber: `${mpn} ${colour}`,
  description: `KCD1 snap-in rocker switch, ${poles === 2 ? "SPST ON-OFF" : "SPDT ON-ON"}, ${colour} rocker, 21 × 15 mm bezel, 19 × 13 mm cutout, ${poles} × 4.8 mm tabs, 6 A 250 VAC`,
  mpn,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("LCSC", `${mpn} rocker switch`),
    supplierSearch("Adafruit", "rocker switch KCD1"),
    supplierSearch("AliExpress", `${mpn} rocker switch 21x15 ${colour}`),
  ],
});
metric("Cutout width", 19.0, "mm");
metric("Cutout height", 13.0, "mm");
metric("Panel thickness max", 2.0, "mm");

connector("Panel", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
connector("Rocker", { at: [0, 0, B + 1.5], axis: [0, 0, 1], x: [1, 0, 0] });
for (let i = 0; i < poles; i++) connector(`Terminal ${i + 1}`, { at: [(i - (poles - 1) / 2) * 5.0, 0, -11.5 - 7.5], axis: [0, 0, -1] });
return parts;
