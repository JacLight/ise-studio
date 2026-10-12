// 2.54 mm (0.1") pin headers, straight through-hole: male pin strips
// (1 or 2 rows) and female socket strips.
//
// Source: the common 2.54 mm header drawing (e.g. Würth 61300●11121 /
// Amphenol FCI 68000 male; Würth 61300●11821 female) — male: 0.64 mm
// square posts, 11.6 mm long, a 2.5 mm insulator, 6.1 mm mating length
// above it and 3.0 mm tail below the board, Ø1.0 mm holes; female: a
// 2.54 × 8.5 mm insulator per row, 1.0 mm square entries, 3.0 mm tails.
//
// Stands on the board (z = 0), pin 1 at the origin, pins along +X, the
// second row (2-row strips) at y = −2.54 (pin 2 opposite pin 1).

const P = 2.54;
const n = Math.round(p.pins);
const rows = Math.round(p.rows);
const female = Math.round(p.kind) === 1;
const W = n * P;
const D = rows * P;
const x0 = -P / 2;
const y1 = P / 2;
const at = [];
for (let i = 0; i < n; i++) for (let r = 0; r < rows; r++) at.push([i * P, -r * P]);

let body;
let posts;
let contacts = null;
if (!female) {
  // Insulator 2.5 mm high with V notches between the pins (where it snaps).
  body = box(W, D, 2.5).translate(x0, y1 - D, 0);
  for (let i = 1; i < n; i++) {
    const x = x0 + i * P;
    body = body.subtract(prismY([[x - 0.25, 2.51], [x, 2.15], [x + 0.25, 2.51]], y1 - D - 0.1, y1 + 0.1));
  }
  posts = union(...at.map(([x, y]) => squarePin(0.64, -3.0, 8.6).translate(x, y, 0)));
} else {
  // Socket strip: 8.5 mm insulator, a 1.0 mm square entry over each contact.
  body = box(W, D, 8.5).translate(x0, y1 - D, 0);
  for (const [x, y] of at) body = body.subtract(box(1.0, 1.0, 7).translate(x - 0.5, y - 0.5, 1.6));
  for (let i = 1; i < n; i++) body = body.subtract(box(0.3, D + 0.2, 0.4).translate(x0 + i * P - 0.15, y1 - D - 0.1, 8.11));
  posts = union(...at.map(([x, y]) => squarePin(0.6, -3.0, 1.7, { tipTop: false }).translate(x, y, 0)));
  // Tulip contacts: a sleeve round each entry's lower part.
  const sleeve = box(1.3, 1.3, 5).translate(-0.65, -0.65, 1.6).subtract(box(0.9, 0.9, 7).translate(-0.45, -0.45, 1));
  contacts = union(...at.map(([x, y]) => sleeve.translate(x, y, 0)));
}
const H = female ? 8.5 : 2.5;
const { parts, massG } = weighed([
  ["Insulator", body, "Polyamide (black)"],
  ["Pins", posts, "Brass, tin-plated"],
  ["Contacts", contacts, "Phosphor bronze, tin-plated"],
]);
const name = `${rows}×${n}`;
const kind = female ? "female" : "male";
stock({
  partNumber: `Pin header 2.54 ${name} ${kind}`,
  description: `2.54 mm pitch ${female ? "female socket" : "male pin"} header, ${rows} row${rows > 1 ? "s" : ""} × ${n}, straight, through-hole`,
  unitPrice: cents(0.02 + rows * n * (female ? 0.012 : 0.006)),
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("LCSC", `2.54mm ${rows}x${n}P ${female ? "female header" : "pin header"}`),
    supplierSearch("Digi-Key", `2.54mm ${rows}x${n} ${female ? "female socket header" : "male header"} through hole`),
    supplierSearch("AliExpress", `2.54mm ${female ? "female" : "male"} pin header ${rows}x${n}`),
  ],
});
metric("Pins", rows * n);
metric("Pitch", P, "mm");
metric("Height", female ? 8.5 : 8.6, "mm");
metric("Hole diameter", 1.0, "mm");

connector("Board", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
connector("Pin 1", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
connector("Mating face", { at: [0, 0, H], axis: [0, 0, 1], x: [1, 0, 0] });
return parts;
