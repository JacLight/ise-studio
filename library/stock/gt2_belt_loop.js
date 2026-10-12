// GT2 closed-loop timing belt (2 mm pitch), 6 mm wide, rubber with
// glass-fibre cords, as it runs round two 20-tooth pulleys.
//
// Sources: Gates PowerGrip GT2 manual — pitch 2 mm, pitch line differential
// 0.254 mm, belt 1.38 mm overall with 0.75 mm teeth (see gt2.js); closed
// loops are sold by pitch length (teeth × 2 mm): 110, 122, 158, 200, 232,
// 280, 300, 400, 610, 852 mm are the usual stock sizes. Mass: the section
// as modelled at 1.35 g/cm³ (neoprene with glass-fibre cords), about
// 8 g per metre, as sellers list (7–9 g/m for 6 mm GT2).
//
// The loop lies in the XY plane, 6 mm wide from z = 0 to 6, wrapped round
// two 20T pulleys (pitch Ø12.73) whose axes are the Z axis and x = C, so
// the centre distance C = (pitch length − 40) / 2: the shape it takes in a
// typical drive. Connectors "Pulley A" and "Pulley B" sit on those axes at
// mid-width, to join a pulley's "Belt plane".

const Lp = p.length;
const teeth = Math.round(Lp / 2);
if (Math.abs(teeth * 2 - Lp) > 1e-6) throw new Error("a GT2 loop's pitch length is a whole number of 2 mm teeth");
const W = 6;
const Rp = gt2Pitch(20) / 2;
const C = (Lp - 2 * Math.PI * Rp) / 2;
if (C <= Rp) throw new Error(`a ${Lp} mm loop is too short to run on two 20T pulleys`);

// The pitch path: along the top from (0, Rp) to (C, Rp), round pulley B
// clockwise, back along the bottom, round pulley A. Position, tangent and
// the normal pointing into the loop at arc length s.
const arc = Math.PI * Rp;
function at(s) {
  s = ((s % Lp) + Lp) % Lp;
  if (s < C) return { P: [s, Rp], T: [1, 0], N: [0, -1] };
  if (s < C + arc) {
    const th = Math.PI / 2 - (s - C) / Rp;
    return { P: [C + Rp * Math.cos(th), Rp * Math.sin(th)], T: [Math.sin(th), -Math.cos(th)], N: [-Math.cos(th), -Math.sin(th)] };
  }
  if (s < 2 * C + arc) return { P: [C - (s - C - arc), -Rp], T: [-1, 0], N: [0, 1] };
  const th = -Math.PI / 2 - (s - 2 * C - arc) / Rp;
  return { P: [Rp * Math.cos(th), Rp * Math.sin(th)], T: [Math.sin(th), -Math.cos(th)], N: [-Math.cos(th), -Math.sin(th)] };
}
/** The point `w` in from the pitch line (negative: out) and `u` along it from s. */
const point = (s, u, w) => {
  const { P, T, N } = at(s);
  return [P[0] + u * T[0] + w * N[0], P[1] + u * T[1] + w * N[1]];
};

// The back: the pitch line offset out by the backing less the pitch line
// differential, sampled finely on the arcs.
const back = [];
const nArc = Math.max(8, Math.ceil(segmentsFor(Rp + 0.4) / 2));
for (const [s0, s1, n] of [[0, C, 1], [C, C + arc, nArc], [C + arc, 2 * C + arc, 1], [2 * C + arc, Lp, nArc]]) {
  for (let i = 0; i < n; i++) back.push(point(s0 + ((s1 - s0) * i) / n, 0, -(GT2_BACK - GT2_PLD)));
}
// The inside: the land 0.254 in from the pitch line, a tooth every 2 mm.
const tooth = gt2ToothPoints().reverse(); // u from −0.6 to +0.6, walking along s
const inside = [];
for (let k = 0; k < teeth; k++) {
  const s = 2 * k + 1;
  for (const [u, w] of tooth) inside.push(point(s, u, GT2_PLD + w));
}
const section = polygon(back).subtract(polygon(inside));
const belt = section.extrude(W);

const massG = section.area * W * 1.35e-3;
stock({
  standard: "GT2 2 mm",
  partNumber: `GT2-${Lp}-6`,
  description: `GT2 closed-loop timing belt, ${Lp} mm pitch length (${teeth} teeth), 6 mm wide, rubber with glass-fibre cords`,
  unitPrice: cents(1.2 + 0.004 * Lp),
  currency: "USD",
  pack: 1,
  massG: Math.round(massG * 100) / 100,
  sources: [
    supplierSearch("Misumi", `timing belt GT2 2mm closed loop ${Lp}`),
    supplierSearch("McMaster-Carr", "timing belts"),
    supplierSearch("AliExpress", `GT2 closed loop belt ${Lp}mm 6mm`),
    supplierSearch("Amazon", `GT2 closed loop timing belt ${Lp}mm 6mm`),
  ],
});
metric("Pitch length", Lp, "mm");
metric("Teeth", teeth);
metric("Width", W, "mm");
metric("Centre distance on 20T", Math.round(C * 1000) / 1000, "mm");

connector("Pulley A", { at: [0, 0, W / 2], axis: [0, 0, 1] });
connector("Pulley B", { at: [C, 0, W / 2], axis: [0, 0, 1] });
return belt;
