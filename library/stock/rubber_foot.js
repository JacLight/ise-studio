// Screw-on rubber foot with a moulded-in steel washer, black NBR rubber.
//
// Source: the common equipment-foot range (generic "rubber feet with steel
// washer" listings, Misumi RBF-style): diameter D at the mounting face,
// height H, tapering to 0.85 D at the floor; a clearance hole for the
// screw, counterbored from the floor side down to the washer, which sits
// 0.4 H in from the mounting face (ISO 7089 washer for the screw).
//
// The mounting face sits on z = 0 against the part it is screwed under,
// the foot hanging down −Z to the floor at z = −H; the screw goes in from
// the floor side, its head on the washer.
// Mass: this nominal geometry, rubber at 1.25 g/cm³, washer at 7.85.

const WASHER = {
  //  d1    d2    h   (ISO 7089)
  3: [3.2, 7.0, 0.5],
  4: [4.3, 9.0, 0.8],
  5: [5.3, 10.0, 1.0],
  6: [6.4, 12.0, 1.6],
};

const D = p.D;
const H = p.H;
const d = p.d;
const [w1, w2, wh] = lookup(WASHER, d, "rubber foot washer");
const R = D / 2;
const c = 0.1 * H; // the floor edge's bevel
const tb = 0.4 * H; // rubber between the mounting face and the washer
const rubber = polygon([[0, 0], [R, 0], [0.85 * R, -H + c], [0.85 * R - c, -H], [0, -H]])
  .revolve(360)
  .subtract(cylinder(H + 2, d / 2 + 0.25).translate(0, 0, -H - 1))
  .subtract(cylinder(H - tb + 0.01, w2 / 2).translate(0, 0, -H - 0.01));
// The washer at the counterbore's bottom.
const washer = polygon([[w1 / 2, -tb - wh], [w2 / 2, -tb - wh], [w2 / 2, -tb], [w1 / 2, -tb]]).revolve(360);

const massG = 1.25e-3 * rubber.volume + 7.85e-3 * washer.volume;
const size = `Ø${num(D)}×${num(H)} M${num(d)}`;
stock({
  partNumber: `Rubber foot ${size} NBR`,
  description: `Screw-on rubber foot Ø${num(D)} × ${num(H)} mm for M${num(d)}, black rubber with moulded-in steel washer`,
  unitPrice: Math.round((0.06 + 0.00006 * D * D * H) * 100) / 100,
  currency: "USD",
  pack: 20,
  massG: Math.round(massG * 100) / 100,
  sources: [
    supplierSearch("McMaster-Carr", "rubber feet"),
    supplierSearch("Misumi", `rubber foot ${num(D)}x${num(H)} M${num(d)}`),
    supplierSearch("Amazon", `rubber feet ${num(D)}x${num(H)}mm steel washer M${num(d)} screw`),
    supplierSearch("AliExpress", `rubber feet ${num(D)}x${num(H)}mm with steel washer`),
  ],
});
metric("Diameter", D, "mm");
metric("Height", H, "mm");
metric("Screw", d, "mm");
metric("Screw seat depth", tb + wh, "mm");

connector("Mount face", { at: [0, 0, 0], axis: [0, 0, 1], part: "Foot" });
connector("Floor contact", { at: [0, 0, -H], axis: [0, 0, -1], part: "Foot" });
connector("Screw seat", { at: [0, 0, -tb - wh], axis: [0, 0, -1], part: "Washer" });
connector("Axis", { at: [0, 0, -H], axis: [0, 0, 1], part: "Foot" });
return [part("Foot", rubber, { material: "NBR rubber" }), part("Washer", washer, { material: "Steel, zinc plated" })];
