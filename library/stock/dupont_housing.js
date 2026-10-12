// "DuPont" 2.54 mm crimp housings (the jumper-wire connector; Harwin M20 /
// FCI Mini-PV style), 1 row of 1 to 8, with the female crimp contacts.
//
// Source: the common 2.54 mm crimp housing drawing (as sold by Pololu,
// items #1900–#1907, and generic "DuPont" kits): 2.5 mm square per
// position (2.54 mm pitch, 0.04 mm short over the strip), 14.0 mm long,
// a 1.0 mm square pin entry over each contact at the mating face, the
// contact's latch window on one side; female contacts 1.5 mm square,
// taking 0.64 mm square posts.
//
// The mating face on z = 0 (the side that goes onto the pins), contact 1
// at the origin, positions along +X, the wire leaving up +Z; the latch
// windows face +Y.

const P = 2.54;
const n = Math.round(p.pins);
const L = 14.0;
const W = (n - 1) * P + 2.5;
let body = box(W, 2.5, L).translate(-1.25, -1.25, 0);
const at = [];
for (let i = 0; i < n; i++) at.push(i * P);
for (const x of at) {
  body = body.subtract(box(1.0, 1.0, 1.0).translate(x - 0.5, -0.5, -0.1)); // pin entry
  body = body.subtract(box(1.7, 1.7, L).translate(x - 0.85, -0.85, 0.8)); // contact cavity, open at the wire end
  body = body.subtract(box(1.1, 0.5, 3.0).translate(x - 0.55, 0.85 - 0.05, 4.0)); // latch window
}
// Female crimp contact: the 1.5 mm box receptacle, then the wire and
// insulation crimps, all as one folded sleeve.
const contact = box(1.5, 1.5, 11.5).translate(-0.75, -0.75, 1.0).subtract(box(1.1, 1.1, 14).translate(-0.55, -0.55, 0))
  .add(box(0.5, 0.25, 2.2).translate(-0.25, 0.75 - 0.01, 4.4)); // the latch lance, into the window
const contacts = union(...at.map((x) => contact.translate(x, 0, 0)));

const { parts, massG } = weighed([
  ["Housing", body, "Polyamide (black)"],
  ["Contacts", contacts, "Phosphor bronze, tin-plated"],
]);
stock({
  partNumber: `DuPont 2.54 1×${n} housing`,
  description: `2.54 mm "DuPont" crimp connector housing, 1 × ${n}, with ${n} female crimp contact${n > 1 ? "s" : ""} (for 0.64 mm square pins)`,
  unitPrice: cents(0.03 + n * 0.025),
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("Pololu", `0.1" 2.54mm crimp connector housing 1x${n}`),
    supplierSearch("AliExpress", `dupont 2.54mm housing 1P ${n}P female terminal`),
    supplierSearch("Amazon", "dupont connector kit 2.54mm"),
  ],
});
metric("Pins", n);
metric("Pitch", P, "mm");
metric("Length", L, "mm");

connector("Mating face", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
connector("Pin 1", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
connector("Wire entry", { at: [((n - 1) * P) / 2, 0, L], axis: [0, 0, 1], x: [1, 0, 0] });
return parts;
