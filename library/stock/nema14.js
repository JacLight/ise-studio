// NEMA 14 hybrid stepper motor, 1.8° (200 steps/rev), body 28 / 34 mm.
//
// Source: NEMA ICS 16 size 14 frame — 35.2 mm square, Ø22 mm × 2 mm pilot,
// M3 tapped holes on a 26 mm square, 3.5 mm deep, Ø5 mm shaft 20 mm from
// the mounting face with a 0.5 mm D-flat over 15 mm; StepperOnline
// datasheets 14HS10-0404S (35×35×28 mm, 0.14 kg) and 14HS13-0804S
// (35×35×34 mm, 0.18 kg). 6-pin JST-PH socket on the rear cap's side, as on
// the NEMA 17s.

const L = p.length;
const MOTORS = {
  // body: [model, mass g, price USD]
  28: ["14HS10-0404S", 140, 11.6],
  34: ["14HS13-0804S", 180, 12.9],
};
const [model, massG, price] = lookup(MOTORS, L, "NEMA 14 body length");
const parts = steppers({
  sq: 35.2, ch: 3.4, chStack: 3.4, cap: 6, pilot: [22, 2], pattern: 26,
  hole: { tap: 3, depth: 3.5 }, shaft: [5, 20, 15, 0.5], leads: "socket",
  bolts: 26, boltHead: 5.2, stackMaterial: "NEMA 14 stator, rotor and windings (lumped)",
}, L);
stock({
  standard: "NEMA 14",
  partNumber: `NEMA14-35x${num(L)}-1.8`,
  description: `NEMA 14 stepper motor 35×35×${num(L)} mm, 1.8°, Ø5 mm D-shaft, 6-pin JST-PH (typ. ${model})`,
  mpn: model,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("StepperOnline", `nema 14 ${model}`),
    supplierSearch("Amazon", `nema 14 stepper motor 35x${num(L)}`),
    supplierSearch("AliExpress", `nema14 stepper motor 35x${num(L)}mm`),
  ],
});
return parts;
