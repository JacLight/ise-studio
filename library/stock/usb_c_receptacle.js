// USB Type-C receptacle, 16-pin (USB 2.0), horizontal top-mount SMT with
// four through-hole shell legs: the part on nearly every USB-C board.
//
// Sources: Korean Hroparts TYPE-C-31-M-12 drawing (LCSC C165948) and GCT
// USB4105-GF-A datasheet — shell 8.94 mm wide × 3.26 mm high (3.31 for the
// GCT) and 7.35 mm long (7.5), the mouth's inside 8.34 × 2.56 mm with full
// round ends (USB Type-C spec 2.1 receptacle: 8.25 × 2.40 mm minimum
// opening, 6.69 × 0.70 mm tongue with 12 contacts a side at 0.5 mm pitch);
// shell legs 0.6 × 0.9 mm (front pair) and 0.6 × 1.4 mm (rear pair) on
// 8.64 mm, 1.6 mm below the board; SMT pads at 0.5 / 0.8 mm pitch behind
// the shell.
//
// The board's top face on z = 0, the mouth facing −Y at the board edge
// (y = 0, the shell protruding 0 mm), the shell running back along +Y, the
// receptacle's centre line on x = 0.

const PARTS = {
  // [model, maker, mpn, shell height, length, price, supplier]
  0: ["TYPE-C-31-M-12", "Korean Hroparts", "TYPE-C-31-M-12", 3.26, 7.35, 0.15, "LCSC"],
  1: ["USB4105-GF-A", "GCT", "USB4105-GF-A", 3.31, 7.5, 0.85, "Digi-Key"],
};
const [model, maker, mpn, H, L, price, where] = PARTS[Math.round(p.model)];
const W = 8.94;
const t = 0.25; // shell sheet
const zc = H / 2;

// A stadium profile run along +Y from y0 to y1, on the receptacle's axis.
const along = (prof, y0, y1) => prof.extrude(y1 - y0).rotate(-90, 0, 0).translate(0, y0, zc);
const shell = along(stadium(W, H).subtract(stadium(W - 2 * t, H - 2 * t)), 0, L)
  .add(along(stadium(W, H), L - t, L)) // closed back
  .add(box(0.6, 0.9, 2.6).translate(-4.32 - 0.3, 0.8, -1.6).add(box(0.6, 0.9, 2.6).translate(4.32 - 0.3, 0.8, -1.6)))
  .add(box(0.6, 1.4, 2.6).translate(-4.32 - 0.3, 4.6, -1.6).add(box(0.6, 1.4, 2.6).translate(4.32 - 0.3, 4.6, -1.6)));
// Insulator: the back block inside the shell and the tongue out to near the mouth.
const block = along(stadium(W - 2 * t - 0.05, H - 2 * t - 0.05), 4.6, L - t + 0.01);
const tongue = box(6.69, 4.6 - 0.9 + 0.01, 0.7).translate(-6.69 / 2, 0.9, zc - 0.35);
const insulator = block.add(tongue);
// Contacts: 12 a side on the tongue (A1–A12 over, B1–B12 under), and the
// SMT leads out of the back onto their pads.
const strips = [];
for (let i = 0; i < 12; i++) {
  const x = -2.75 + i * 0.5;
  for (const z of [zc + 0.35, zc - 0.35 - 0.05]) strips.push(box(0.25, 2.6, 0.05).translate(x - 0.125, 1.2, z));
}
const leads = [];
const pads = [-3.2, -2.4, -1.75, -1.25, -0.75, -0.25, 0.25, 0.75, 1.25, 1.75, 2.4, 3.2];
for (const x of pads) leads.push(box(0.3, 1.2, 0.15).translate(x - 0.15, L - 0.2, 0));
const contacts = union(...strips, ...leads);

const { parts, massG } = weighed([
  ["Shell", shell, "Stainless steel"],
  ["Insulator", insulator, "LCP (black)"],
  ["Contacts", contacts, "Phosphor bronze, gold-plated"],
]);
stock({
  standard: "USB Type-C 2.1",
  partNumber: model,
  description: `USB Type-C receptacle, 16 pins (USB 2.0), horizontal top-mount SMT with through-hole shell legs, ${num(W)} × ${num(H)} × ${num(L)} mm (${maker} ${mpn})`,
  manufacturer: maker,
  mpn,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch(where, mpn),
    supplierSearch("Mouser", mpn),
    supplierSearch("AliExpress", "USB type-C 16pin SMT receptacle"),
  ],
});
metric("Shell width", W, "mm");
metric("Shell height", H, "mm");
metric("Shell length", L, "mm");
metric("Plug opening", 8.34, "mm");

connector("Board", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
connector("Mating face", { at: [0, 0, zc], axis: [0, -1, 0], x: [1, 0, 0] });
connector("Board edge", { at: [0, 0, 0], axis: [0, -1, 0], x: [1, 0, 0] });
return parts;
