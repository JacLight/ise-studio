// Cast corner bracket (L bracket with side gussets) for T-slot extrusion:
// 20 series (2020, 2040), 30 series (3030, 3060), 40 series (4040, 4080),
// die-cast aluminium.
//
// Sources: the cast corner brackets sold with these profiles (Misumi HBLFS
// pattern, the generic "2020 corner bracket 20×20×20"): leg length a, width
// w (the profile's face), leg thickness t, side gussets, one hole per leg
// (two on the long brackets) for the series' screw (M5 Ø5.5, M6 Ø6.6), and
// short locating keys under each leg that sit in the slot opening, kept
// clear of the T-nut under each hole. Mass: the modelled casting (ADC12,
// 2.74 g/cm³; sellers list 7–9 g for 2020, 20–30 g for 3030).
//
// The inner corner on the origin: leg A lies on the XY plane (its seat, the
// face that bears on extrusion A, on z = 0, running out along +X), leg B
// stands on the YZ plane (its seat on x = 0, running up +Z), width along Y
// centred. Each hole's connector is on the leg's outer face, where a screw
// head seats.

//            series  a   w   t   gusset hole holes USD
const BRACKET = {
  2020: [20, 20, 20, 3.5, 2, 5.5, 1, 0.35],
  2040: [20, 40, 20, 4, 2, 5.5, 2, 0.6],
  3030: [30, 30, 30, 5, 2.5, 6.6, 1, 0.7],
  3060: [30, 60, 30, 5, 2.5, 6.6, 2, 1.1],
  4040: [40, 40, 40, 6, 3, 6.6, 1, 1.1],
  4080: [40, 80, 40, 6, 3, 6.6, 2, 1.8],
};
const size = p.size;
const [series, a, w, t, tg, hole, nHoles, price] = lookup(BRACKET, size, "corner bracket");
const s = lookup(TSLOT_SERIES, series, "T-slot series");
const key = s.slot - 0.4; // locating key width
const hk = Math.min(1.5, s.lip - 0.3); // key height

// One leg in the XZ plane (x along the leg, z through it), extruded across Y.
const sideProfile = polygon([[0, 0], [a, 0], [a, t], [t, t], [t, a], [0, a]]);
const legs = sideProfile.extrude(w).rotate(90, 0, 0).translate(0, w / 2, 0);
// Gussets: triangles at both edges from leg to leg.
const tri = polygon([[t - 0.01, t - 0.01], [a, t - 0.01], [a, t + 0.6], [t + 0.6, a], [t - 0.01, a]]);
const gussets = [w / 2, -w / 2 + tg].map((y) => tri.extrude(tg).rotate(90, 0, 0).translate(0, y, 0));
// Locating keys: short tabs near both ends of each leg's seat.
const tabs = [];
for (const [x0, x1] of [[t + 0.5, t + 3], [a - 3, a - 0.5]]) {
  tabs.push(box(x1 - x0, key, hk + 0.01).translate(x0, -key / 2, -hk)); // under leg A
  tabs.push(box(hk + 0.01, key, x1 - x0).translate(-hk, -key / 2, x0)); // behind leg B
}
let bracket = union(legs, ...gussets, ...tabs);

// Holes, on the leg's centre line.
const xs = nHoles === 1 ? [t + (a - t) / 2] : [t + (a - t) * 0.25, t + (a - t) * 0.75];
const holes = [];
for (const x of xs) {
  holes.push(cylinder(t + 2 * hk + 2, hole / 2).translate(x, 0, -hk - 1));
  holes.push(cylinder(t + 2 * hk + 2, hole / 2).rotate(0, 90, 0).translate(-hk - 1, 0, x));
}
bracket = bracket.subtract(union(holes));

const massG = bracket.volume * 2.74e-3;
stock({
  partNumber: `BRKT-${size}-CAST`,
  description: `Corner bracket ${size}, cast aluminium, ${a}×${a}×${w} mm, ${nHoles * 2}× Ø${hole} for ${series} series extrusion`,
  unitPrice: price,
  currency: "USD",
  pack: 10,
  massG: Math.round(massG * 10) / 10,
  sources: [
    supplierSearch("Misumi", `bracket ${series} series extrusion ${size}`),
    supplierSearch("McMaster-Carr", "t-slotted framing brackets"),
    supplierSearch("AliExpress", `${size} corner bracket aluminum extrusion`),
    supplierSearch("Amazon", `${size} corner bracket L shape`),
  ],
});
metric("Leg", a, "mm");
metric("Width", w, "mm");
metric("Thickness", t, "mm");
metric("Hole", hole, "mm");

connector("Seat A", { at: [a / 2, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
connector("Seat B", { at: [0, 0, a / 2], axis: [-1, 0, 0], x: [0, 0, 1] });
xs.forEach((x, i) => {
  const tag = nHoles === 1 ? "" : ` ${i + 1}`;
  connector(`Hole A${tag}`, { at: [x, 0, t], axis: [0, 0, 1] });
  connector(`Hole B${tag}`, { at: [t, 0, x], axis: [1, 0, 0], x: [0, 0, 1] });
});
return bracket;
