// MTS mini toggle switches (6 mm bushing): MTS-101 SPST ON-OFF, MTS-102
// SPDT ON-ON, MTS-103 SPDT ON-OFF-ON, MTS-202 DPDT ON-ON.
//
// Source: MTS-1xx / MTS-2xx series drawing (Salecom / generic, as sold by
// LCSC and Adafruit #3219): body 12.7 × 7.9 mm (12.7 × 12.7 for the
// double-pole), 10.0 mm deep; M6 × 0.75 bushing 9.0 mm long with a keyway,
// for a Ø6.2 mm panel hole; bat lever Ø3.0 → Ø2.4 mm, 10.5 mm above the
// bushing, thrown 12°; solder lugs 2.0 × 0.5 mm, 5.0 mm long, on 4.7 mm
// centres; an M6 hex nut (8 mm across flats, 2 mm) and a lock washer.
//
// The panel's back face on z = 0 (the body's top seats on it), bushing and
// lever up +Z through the panel, body and lugs below (−Z); lugs along X.

const MODELS = {
  // [mpn, poles, positions, terminals per row, price]
  0: ["MTS-101", 1, "ON-OFF", 2, 0.3],
  1: ["MTS-102", 1, "ON-ON", 3, 0.32],
  2: ["MTS-103", 1, "ON-OFF-ON", 3, 0.36],
  3: ["MTS-202", 2, "ON-ON", 3, 0.55],
};
const [mpn, poles, positions, perRow, price] = MODELS[Math.round(p.model)];
const BL = 12.7;
const BW = poles === 2 ? 12.7 : 7.9;
const BD = 10.0;
// Body: the case with its metal frame round the top edge.
const body = box(BL, BW, BD - 0.6).translate(-BL / 2, -BW / 2, -BD);
const frame = box(BL, BW, 0.6).translate(-BL / 2, -BW / 2, -0.6)
  .add(box(0.4, BW, 4).translate(-BL / 2 - 0.01, -BW / 2, -4.2)).add(box(0.4, BW, 4).translate(BL / 2 - 0.39, -BW / 2, -4.2));
// Bushing: M6 × 0.75 thread with the keyway, the lever's pivot in its bore.
const bushing = externalThread({ d: 6, pitch: 0.75, z0: 0, z1: 9, tip: "top" })
  .subtract(box(1.0, 1.2, 9.2).translate(-0.5, 3 - 0.6, -0.1))
  .subtract(cylinder(9.2, 1.9).translate(0, 0, 0.5));
const throwDeg = positions === "ON-OFF-ON" ? 0 : 12;
const lever = cylinder(10.5 + 2, 1.5, { top: 1.2 }).translate(0, 0, -2).add(sphere(1.2).translate(0, 0, 10.5))
  .rotate(0, throwDeg, 0).translate(0, 0, 9);
// Hardware: the hex nut and the lock washer on the bushing, as fitted to a
// panel up to 3 mm thick.
const nut = hexPrism(8, 2).subtract(cylinder(3, 3.05).translate(0, 0, -0.5)).translate(0, 0, 3.6);
const washer = cylinder(0.5, 4.5).subtract(cylinder(1, 3.1).translate(0, 0, -0.2)).translate(0, 0, 3.1);
// Lugs: solder lugs out of the base, with their wire holes.
const lugs = [];
const at = [];
for (let r = 0; r < poles; r++) {
  for (let i = 0; i < perRow; i++) {
    const x = perRow === 2 ? (i - 0.5) * 4.7 : (i - 1) * 4.7;
    const y = poles === 2 ? (r - 0.5) * 4.7 : 0;
    at.push([x, y]);
    lugs.push(box(2.0, 0.5, 5.5).translate(x - 1.0, y - 0.25, -BD - 5.0).subtract(cylinder(1, 0.55).rotate(90, 0, 0).translate(x, y + 0.5, -BD - 3.3)));
  }
}
const { parts, massG } = weighed([
  ["Body", body, "Polyamide (black)"],
  ["Frame", frame, "Steel, nickel-plated"],
  ["Bushing", bushing, "Brass, tin-plated"],
  ["Lever", lever, "Steel, nickel-plated"],
  ["Nut", nut, "Brass, tin-plated"],
  ["Washer", washer, "Steel, nickel-plated"],
  ["Lugs", union(...lugs), "Brass, tin-plated"],
]);
stock({
  partNumber: mpn,
  description: `Mini toggle switch ${mpn}, ${poles === 2 ? "DPDT" : perRow === 2 ? "SPST" : "SPDT"} ${positions}, M6 bushing (Ø6.2 mm panel hole), solder lugs, 6 A 125 VAC`,
  mpn,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("LCSC", `${mpn} toggle switch`),
    supplierSearch("Adafruit", "panel mount mini toggle switch"),
    supplierSearch("AliExpress", `${mpn} mini toggle switch 6mm`),
  ],
});
metric("Panel hole", 6.2, "mm");
metric("Bushing length", 9.0, "mm");
metric("Lugs", at.length);

connector("Panel", { at: [0, 0, 0], axis: [0, 0, 1], x: [1, 0, 0] });
connector("Lever", { at: [0, 0, 9], axis: [0, 0, 1], x: [1, 0, 0] });
at.forEach(([x, y], i) => connector(`Lug ${i + 1}`, { at: [x, y, -BD - 5.0], axis: [0, 0, -1] }));
return parts;
