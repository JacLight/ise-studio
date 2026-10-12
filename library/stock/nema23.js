// NEMA 23 hybrid stepper motor, 1.8° (200 steps/rev), body 41 / 56 / 76 mm.
//
// Source: NEMA ICS 16 size 23 frame — 56.4 mm square flange, Ø38.1 mm ×
// 1.6 mm pilot, four Ø5.1 mm through holes on a 47.14 mm square in 4.8 mm
// ears (the body behind relieved at the corners so M5 screws come in from
// the back), Ø6.35 mm (¼") shaft 21 mm from the mounting face with a 0.5 mm
// D-flat over 15 mm (Ø8 mm on the long high-torque bodies); StepperOnline
// datasheets 23HS16-2004S (41 mm, 0.45 kg), 23HS22-2804S (56 mm, 0.7 kg),
// 23HS30-2804S (76 mm, 1.0 kg), 23HS30-3004D-style Ø8 shaft. Four flying
// leads (black, green, red, blue) out of a moulded relief on the rear cap.

const L = p.length;
const sd = p.shaft;
const MOTORS = {
  // body: [model, mass g, price USD]
  41: ["23HS16-2004S", 450, 15.9],
  56: ["23HS22-2804S", 700, 18.5],
  76: ["23HS30-2804S", 1000, 23.9],
};
const [model, massG, base] = lookup(MOTORS, L, "NEMA 23 body length");
const parts = steppers({
  sq: 56.4, ch: 1.5, chStack: 15.5, cap: 10, pilot: [38.1, 1.6], pattern: 47.14,
  hole: { through: 5.1, flange: 4.8 }, shaft: [sd, 21, 15, 0.5], leads: "wires",
  bolts: 36, boltHead: 5.6, stackMaterial: "NEMA 23 stator, rotor and windings (lumped)",
}, L);
const price = cents(base + (sd === 8 ? 2.5 : 0));
const shaftName = sd === 8 ? "Ø8" : "Ø6.35";
stock({
  standard: "NEMA 23",
  partNumber: `NEMA23-56x${num(L)}-1.8-${num(sd)}`,
  description: `NEMA 23 stepper motor 56×56×${num(L)} mm, 1.8°, ${shaftName} mm D-shaft, 4 flying leads (typ. ${model})`,
  mpn: sd === 8 ? undefined : model,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG: massG + (sd === 8 ? 10 : 0),
  sources: [
    supplierSearch("StepperOnline", `nema 23 ${model}`),
    supplierSearch("Amazon", `nema 23 stepper motor 56x${num(L)} ${shaftName}mm shaft`),
    supplierSearch("AliExpress", `nema23 stepper motor 57x${num(L)}mm ${shaftName}`),
  ],
});
return parts;
