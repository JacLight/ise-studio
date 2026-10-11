// Set collar, DIN 705 form A (adjusting ring with a set screw), zinc-plated
// steel, with its DIN 916 / ISO 4029 hexagon socket set screw (cup point).
//
// Sources: DIN 705 table — bore d, outside D, width b and set-screw thread
// (d 5: D 10, b 6, M3; d 6: 12, 8, M4; d 8: 16, 8, M5; d 10: 20, 10, M6;
// d 12: 22, 12, M6); ISO 4029 socket sizes s (M3 1.5, M4 2, M5 2.5, M6 3).
//
// Axis along +Z, face A on z = 0, face B on z = b; the tapped hole runs
// along +X through the wall at mid-width, its set screw flush with the
// outside and its cup point at the bore.

const COLLAR = { 5: [10, 6, 3], 6: [12, 8, 4], 8: [16, 8, 5], 10: [20, 10, 6], 12: [22, 12, 6] };
const SOCKET = { 3: 1.5, 4: 2, 5: 2.5, 6: 3 };
const PRICE = { 5: 0.9, 6: 0.9, 8: 1.0, 10: 1.3, 12: 1.5 };

const d = p.d;
const [D, b, M] = lookup(COLLAR, d, "DIN 705");
const pitch = coarsePitch(M);
const wall = (D - d) / 2;
const c = 0.3;

const ring = polygon([[d / 2 + c, 0], [D / 2 - c, 0], [D / 2, c], [D / 2, b - c], [D / 2 - c, b], [d / 2 + c, b], [d / 2, b - c], [d / 2, c]]).revolve(360);
// Radial tapped hole: built along +Z, turned onto +X (rotate about Y), at mid-width.
const radial = (s) => s.rotate(0, 90, 0).translate(0, 0, b / 2);
const hole = radial(internalThreadTool({ d: M, pitch, z0: d / 2 - 0.5, z1: D / 2 + 0.5 }));
const collar = ring.subtract(hole);

// The set screw: M × wall, cup point at the bore, flush with the outside.
const s = lookup(SOCKET, M, "set screw socket");
const l = wall;
const screw = radial(
  externalThread({ d: M, pitch, z0: 0, z1: l, tip: "bottom" })
    .subtract(hexSocket(s, 0.6 * l, l))
    .translate(0, 0, D / 2 - l),
);

const rho = 7.85e-3;
// Mass: the nominal ring (the set screw fills its hole).
const massG = rho * (Math.PI / 4) * (D * D - d * d) * b;
stock({
  standard: "DIN 705 A",
  partNumber: `DIN 705 A ${num(d)}×${num(D)}×${num(b)} M${num(M)}`,
  description: `Set collar ${num(d)} mm bore, Ø${num(D)}×${num(b)}, zinc-plated steel, DIN 916 M${num(M)} set screw`,
  unitPrice: lookup(PRICE, d, "price"),
  currency: "USD",
  pack: 1,
  massG: Math.round(massG * 100) / 100,
  sources: [
    supplierSearch("McMaster-Carr", "shaft collars"),
    supplierSearch("Misumi", `set collar ${num(d)}mm set screw`),
    supplierSearch("AliExpress", `DIN 705 shaft collar ${num(d)}mm`),
  ],
});
metric("Bore", d, "mm");
metric("Set screw", M, "mm");

connector("Bore", { at: [0, 0, b / 2], axis: [0, 0, 1] });
connector("Face A", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Face B", { at: [0, 0, b], axis: [0, 0, 1] });
connector("Set screw", { at: [D / 2, 0, b / 2], axis: [1, 0, 0], x: [0, 0, 1] });

return [
  part("Collar", collar, { material: "Steel, zinc plated" }),
  part("Set screw", screw, { material: "Steel 45H" }),
];
