// Hex standoff (PCB spacer), brass, nickel-free: female–female (tapped
// through) or male–female (a threaded stud at one end).
//
// Source: the common brass standoff ranges (Wurth WA-SSTII / Keystone /
// generic listings): body across flats M2 3.5 mm, M2.5 and M3 5 mm; stud
// length 3.5 mm (M2), 5 mm (M2.5) and 6 mm (M3); the body length L is the
// spacing it makes, stud not counted; both ends of the hex chamfered
// 0.2 mm; the tapped bore ISO 68-1, countersunk at its mouths.
//
// Stands on its bottom face at z = 0, the body up to z = L; a male–female
// stud runs down −Z from the bottom face, like a screw's shank.
// Mass: this nominal geometry in brass CuZn39Pb3 (8.47 g/cm³).

const STANDOFF = {
  //    af   stud
  2: [3.5, 3.5],
  2.5: [5.0, 5.0],
  3: [5.0, 6.0],
};
// Typical price of one standoff in a pack of 50 (brass, USD): base + per mm.
const PRICE = { 2: [0.05, 0.003], 2.5: [0.05, 0.003], 3: [0.05, 0.003] };

const d = p.d;
const L = p.length;
const male = p.male >= 0.5;
const [af, stud] = lookup(STANDOFF, d, "standoff");
const pitch = coarsePitch(d);
const real = p.threads >= 0.5;

// Hex body with its corners broken at both ends (a 45° bevel of 0.2 mm on
// the circumscribed corners).
const e = af / Math.sqrt(3);
const c = 0.2;
const body = hexPrism(af, L).intersect(polygon([[0, 0], [e - c, 0], [e, c], [e, L - c], [e - c, L], [0, L]]).revolve(360));
// The tapped bore: right through a female–female; from the top down to
// 1 mm above the stud for a male–female. A 90° countersink at each mouth.
const depth = male ? L - 1 : L;
const sink = 0.4 * pitch;
const cs = d / 2 + 0.05;
let standoff = body
  .subtract(internalThreadTool({ d, pitch, z0: L - depth, z1: L, real }).intersect(cylinder(depth + 0.02, d).translate(0, 0, L - depth - 0.01)))
  .subtract(cylinder(sink + 0.01, cs - sink - 0.01, { top: cs }).translate(0, 0, L - sink));
if (male) {
  // A drill point closing the blind bore, then the stud.
  const minorR = d / 2 - (5 * Math.sqrt(3) / 16) * pitch;
  standoff = standoff
    .subtract(cylinder(minorR / Math.tan(rad(59)), 0.001, { top: minorR }).translate(0, 0, L - depth - minorR / Math.tan(rad(59)) + 0.001))
    .add(externalThread({ d, pitch, z0: -stud, z1: 0.01, real, tip: "bottom" }));
} else {
  standoff = standoff.subtract(cylinder(sink + 0.01, cs, { top: cs - sink - 0.01 }).translate(0, 0, -0.01));
}

const massG = 8.47e-3 * standoff.volume;
const [base, perMm] = lookup(PRICE, d, "price");
const type = male ? "M-F" : "F-F";
const size = male ? `M${num(d)}×${num(L)}+${num(stud)}` : `M${num(d)}×${num(L)}`;
stock({
  partNumber: `Standoff ${type} ${size} brass`,
  description: `Hex standoff ${male ? "male–female" : "female–female"} M${num(d)}, ${num(L)} mm body${male ? ` + ${num(stud)} mm stud` : ""}, ${num(af)} mm hex, brass`,
  unitPrice: Math.round((base + perMm * (L + (male ? stud : 0))) * 1000) / 1000,
  currency: "USD",
  pack: 50,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "standoffs"),
    supplierSearch("Misumi", `hex standoff brass M${num(d)} ${num(L)}mm ${male ? "male female" : "female female"}`),
    supplierSearch("Amazon", `M${num(d)} brass standoff ${num(L)}mm ${type}`),
    supplierSearch("AliExpress", `M${num(d)} brass hex standoff ${num(L)}mm ${male ? "male female" : "female female"}`),
  ],
});
metric("Across flats", af, "mm");
metric("Spacing", L, "mm");
metric("Tapped thread", d, "mm");
metric("Tapped depth", depth, "mm");
if (male) metric("Stud length", stud, "mm");

// The faces it stands between, the axis, and (male–female) the stud's tip.
connector("Bottom face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top face", { at: [0, 0, L], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
if (male) connector("Stud tip", { at: [0, 0, -stud], axis: [0, 0, -1] });
return standoff;
