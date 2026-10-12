// T8 lead screw nut: the brass flange nut, or the POM anti-backlash nut
// (spring-loaded floating half keyed to the flanged half), for Tr8 screws
// of lead 2, 4 or 8 mm.
//
// Sources: the T8 nuts sold with 3D-printer lead screws (generic, matching
// the dimensions sellers publish): brass — flange Ø22 × 3.5, body Ø10.2,
// overall 15 mm, 4 × Ø3.5 holes on a Ø16 bolt circle; POM anti-backlash —
// the same flange and bolt circle, body Ø10.2, overall 24 mm with a Ø14
// floating nut on two guide fingers and a steel spring between. Internal
// thread: the Tr8 profile (leadscrew.js) with 0.1 mm clearance. Mass: the
// modelled parts (brass 8.5, POM 1.41 g/cm³; sellers list ≈ 12 g brass,
// 4–5 g POM).
//
// Axis along +Z, the flange's outer face on z = 0, the flange seat (the face
// a carriage plate bears on) at z = 3.5, the body running up +Z.

const lead = p.lead;
const starts = Math.round(lead / 2);
if (![1, 2, 4].includes(starts)) throw new Error("T8 nuts come for lead 2, 4 or 8 mm");
const antiBacklash = p.type >= 0.5;
const Df = 22;
const tf = 3.5;
const Db = 10.2;
const pcd = 16;
const hole = 3.5;
const H = antiBacklash ? 24 : 15;
const c = 0.3;
const holes = [45, 135, 225, 315].map((a) => [(pcd / 2) * Math.cos(rad(a)), (pcd / 2) * Math.sin(rad(a))]);

// The flanged nut: flange with its bolt holes, body, the thread.
const zBody = antiBacklash ? 13 : H;
const flange = circle(Df / 2).subtract(...holes.map(([x, y]) => circle(hole / 2).translate(x, y))).extrude(tf);
const bodyOutline = polygon([[0, tf - 0.01], [Db / 2, tf - 0.01], [Db / 2, zBody - c], [Db / 2 - c, zBody], [0, zBody]]).revolve(360);
let nut = flange.add(bodyOutline).subtract(trNutTool({ d: 8, pitch: 2, z0: 0, z1: zBody }));

const parts = [];
if (antiBacklash) {
  // Two guide fingers up from the body (arcs at r 5.5…7, ±25° about ±X).
  const finger = (a0) => {
    const pts = [];
    for (let i = 0; i <= 10; i++) pts.push([5.5 * Math.cos(rad(a0 - 25 + 5 * i)), 5.5 * Math.sin(rad(a0 - 25 + 5 * i))]);
    for (let i = 10; i >= 0; i--) pts.push([7 * Math.cos(rad(a0 - 25 + 5 * i)), 7 * Math.sin(rad(a0 - 25 + 5 * i))]);
    return polygon(pts);
  };
  const fingers = finger(0).add(finger(180));
  // Fingers rise from a collar at the top of the body.
  const collar = polygon([[4.3, zBody - 0.01], [7, zBody - 0.01], [7, zBody + 1.5], [4.3, zBody + 1.5]]).revolve(360);
  nut = nut.add(collar, fingers.extrude(H - zBody - 1.5).translate(0, 0, zBody + 1.49));
  // The floating nut: Ø14 × 6, slotted for the fingers, threaded.
  const zf = H - 6;
  const slots = finger(0).offset(0.15).add(finger(180).offset(0.15));
  const floating = circle(7).subtract(slots).extrude(6).translate(0, 0, zf).subtract(trNutTool({ d: 8, pitch: 2, z0: zf, z1: H }));
  // The spring between collar and floating nut: Ø0.8 wire on a Ø9.6 coil.
  const z0 = zBody + 1.5;
  const coils = 4;
  const len = zf - z0;
  const wire = 0.4;
  const rc = 4.8;
  const perTurn = segmentsFor(rc);
  const ring = 8;
  const positions = [];
  const steps = coils * perTurn;
  const lift = (len - 2 * wire - 0.1) / coils;
  for (let s = 0; s <= steps; s++) {
    const a = (2 * Math.PI * s) / perTurn;
    const zc = z0 + wire + 0.05 + (s / perTurn) * lift;
    const u = [Math.cos(a), Math.sin(a), 0];
    for (let j = 0; j < ring; j++) {
      const b = (2 * Math.PI * j) / ring;
      const rr = rc + wire * Math.cos(b);
      positions.push([rr * u[0], rr * u[1], zc + wire * Math.sin(b)]);
    }
  }
  const at = (s, j) => s * ring + j;
  const triangles = [];
  for (let s = 0; s < steps; s++) {
    for (let j = 0; j < ring; j++) {
      const q = (j + 1) % ring;
      triangles.push([at(s, j), at(s + 1, j), at(s + 1, q)], [at(s, j), at(s + 1, q), at(s, q)]);
    }
  }
  for (let j = 1; j < ring - 1; j++) {
    triangles.push([at(0, 0), at(0, j), at(0, j + 1)]);
    triangles.push([at(steps, 0), at(steps, j + 1), at(steps, j)]);
  }
  parts.push(part("Floating nut", floating, { material: "Acetal (POM)" }));
  parts.push(part("Spring", fromMesh(positions, triangles), { material: "Spring steel" }));
}
const material = antiBacklash ? "Acetal (POM)" : "Brass CuZn39Pb3";
parts.unshift(part("Nut", nut, { material }));

const volume = parts.reduce((v, x) => v + x.solid.volume * { "Acetal (POM)": 1.41e-3, "Brass CuZn39Pb3": 8.5e-3, "Spring steel": 7.85e-3 }[x.material], 0);
const kind = antiBacklash ? "anti-backlash nut, POM" : "flange nut, brass";
stock({
  partNumber: `T8-L${lead}-${antiBacklash ? "AB-POM" : "BRASS"}`,
  description: `Lead screw ${kind}, Tr8 lead ${lead} mm, flange Ø${Df}, 4× Ø${hole} on Ø${pcd}`,
  unitPrice: antiBacklash ? 2.2 : 1.6,
  currency: "USD",
  pack: 1,
  massG: Math.round(volume * 10) / 10,
  sources: [
    supplierSearch("AliExpress", `T8 ${antiBacklash ? "anti backlash nut POM" : "brass nut"} lead ${lead}mm`),
    supplierSearch("Amazon", `T8 lead screw ${antiBacklash ? "anti-backlash nut" : "brass nut"} ${lead}mm lead`),
    supplierSearch("Misumi", "lead screw nut flanged 8mm"),
  ],
});
metric("Lead", lead, "mm");
metric("Flange diameter", Df, "mm");
metric("Bolt circle", pcd, "mm");
metric("Bolt hole", hole, "mm");
metric("Length", H, "mm");

connector("Bore", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("Flange face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Flange seat", { at: [0, 0, tf], axis: [0, 0, 1] });
holes.forEach(([x, y], i) => connector(`Hole ${i + 1}`, { at: [x, y, 0], axis: [0, 0, -1] }));
return parts;
