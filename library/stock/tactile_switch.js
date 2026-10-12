// Tactile push switches, through-hole: the 6 × 6 mm family in its four
// common heights (4.3, 5.0, 7.0, 9.5 mm) and the 12 × 12 × 7.3 mm.
//
// Sources: Omron B3F datasheet — 6 × 6 mm body 3.5 mm high under its
// steel cover, Ø3.5 mm round plunger, four legs on a 6.5 × 4.5 mm pattern
// (Ø1.0 mm holes), heights to the plunger top 4.3 (B3F-1000), 5.0, 7.0 and
// 9.5 mm across the B3F / generic TS-66 lines; 12 × 12 mm: body 3.8 mm,
// Ø6.8 mm plunger(ish) 7.3 mm to the top, legs on 12.5 × 5.0 mm (B3F-4000
// class).
//
// Centred on the origin, the board's top face on z = 0, legs down −Z; the
// legs' pairs (shorted in pairs inside) run along X.

const SIZES = {
  // key: [body, body height, plunger Ø, top, leg pattern X, Y, leg w, price, mpn]
  "6×6×4.3": [6, 3.5, 3.5, 4.3, 6.5, 4.5, 0.7, 0.06, "B3F-1000"],
  "6×6×5": [6, 3.5, 3.5, 5.0, 6.5, 4.5, 0.7, 0.06, undefined],
  "6×6×7": [6, 3.5, 3.5, 7.0, 6.5, 4.5, 0.7, 0.07, undefined],
  "6×6×9.5": [6, 3.5, 3.5, 9.5, 6.5, 4.5, 0.7, 0.08, undefined],
  "12×12×7.3": [12, 3.8, 6.8, 7.3, 12.5, 5.0, 1.0, 0.18, undefined],
};
const key = `${num(p.body)}×${num(p.body)}×${num(p.height)}`;
const [S, bh, pd, top, lx, ly, lw, price, mpn] = lookup(SIZES, key, "tactile switch");

// Body: the moulded base, a hair inside the cover, on four 0.3 mm feet.
let body = box(S - 0.2, S - 0.2, bh - 0.3 - 0.3).translate(-(S - 0.2) / 2, -(S - 0.2) / 2, 0.3);
for (const [sx, sy] of [[-1, -1], [1, -1], [1, 1], [-1, 1]]) body = body.add(cylinder(0.31, 0.5).translate(sx * (S / 2 - 0.9), sy * (S / 2 - 0.9), 0));
// Steel cover: a plate over the top with the two side straps clipping the base.
const cover = box(S, S, 0.3).translate(-S / 2, -S / 2, bh - 0.3)
  .subtract(cylinder(1, pd / 2 + 0.25).translate(0, 0, bh - 0.5))
  .add(box(S * 0.4, 0.3, bh - 0.8).translate(-S * 0.2, -S / 2, 0.5))
  .add(box(S * 0.4, 0.3, bh - 0.8).translate(-S * 0.2, S / 2 - 0.3, 0.5));
// Plunger: round stem up from inside the body to its top.
const plunger = cylinder(top - bh + 1.0, pd / 2).translate(0, 0, bh - 1.0);
// Legs: flat formed legs out of the base's sides, down through the board.
const legs = [];
for (const sx of [-1, 1]) {
  for (const sy of [-1, 1]) {
    const x = (sx * lx) / 2;
    const y = (sy * ly) / 2;
    legs.push(box(Math.abs(x) - S / 2 + 0.3 + lw / 2, lw, 0.3).translate(sx > 0 ? S / 2 - 0.3 : x - lw / 2, y - lw / 2, 0.6));
    legs.push(box(0.3, lw, 3.5 + 0.9).translate(x - 0.15, y - lw / 2, -3.5));
  }
}
const { parts, massG } = weighed([
  ["Base", body, "PBT (black)"],
  ["Cover", cover, "Steel, nickel-plated"],
  ["Plunger", plunger, "Polyamide (black)"],
  ["Legs", union(...legs), "Phosphor bronze, tin-plated"],
]);
stock({
  partNumber: `Tactile switch ${key}`,
  description: `Tactile push switch, ${num(S)} × ${num(S)} mm, ${num(top)} mm high, through-hole, SPST momentary${mpn ? ` (Omron ${mpn})` : ""}`,
  manufacturer: mpn ? "Omron" : undefined,
  mpn,
  unitPrice: mpn ? 0.28 : price,
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch(mpn ? "Digi-Key" : "LCSC", mpn ?? `tactile switch ${num(S)}x${num(S)}x${num(top)} THT`),
    supplierSearch("AliExpress", `tactile push button ${num(S)}x${num(S)}x${num(top)}mm`),
  ],
});
metric("Height", top, "mm");
metric("Body", S, "mm");
metric("Leg pattern X", lx, "mm");
metric("Leg pattern Y", ly, "mm");

connector("Board", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
connector("Plunger", { at: [0, 0, top], axis: [0, 0, 1], x: [1, 0, 0] });
[[-1, 1], [1, 1], [-1, -1], [1, -1]].forEach(([sx, sy], i) => connector(`Pin ${i + 1}`, { at: [(sx * lx) / 2, (sy * ly) / 2, 0], axis: [0, 0, -1] }));
return parts;
