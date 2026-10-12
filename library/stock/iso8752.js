// Spring pin (roll pin), slotted, heavy duty, ISO 8752 (DIN 1481), spring
// steel hardened 420–520 HV.
//
// Source: ISO 8752:2009 table 1 — nominal diameter d (the hole it goes in,
// H12), outside diameter before fitting d1 (the mean of min and max), wall
// thickness s, chamfer length a (max); both ends chamfered. The slot as
// rolled, about a quarter of d wide when free.
//
// Stands on one end at z = 0, axis along +Z, the other end at z = l; the
// slot at +X.
// Mass: this nominal geometry at 7.85 g/cm³.

const ISO8752 = {
  //    d1     s     a
  2: [2.35, 0.4, 0.4],
  2.5: [2.85, 0.5, 0.45],
  3: [3.4, 0.6, 0.45],
  4: [4.5, 0.8, 0.7],
  5: [5.5, 1.0, 0.7],
  6: [6.55, 1.2, 0.9],
  8: [8.65, 1.5, 1.8],
};
// Typical price of one pin in a pack of 100 (spring steel, USD): base + per mm.
const PRICE = { 2: [0.02, 0.001], 2.5: [0.02, 0.001], 3: [0.02, 0.0012], 4: [0.025, 0.0015], 5: [0.03, 0.002], 6: [0.035, 0.0025], 8: [0.05, 0.0035] };

const d = p.d;
const L = p.length;
const [d1, s, a] = lookup(ISO8752, d, "ISO 8752");
const ro = d1 / 2;
const ri = ro - s;
const slot = 0.25 * d;

// The rolled section: a ring less the slot, extruded the pin's length.
const section = circle(ro).subtract(circle(ri)).subtract(rect(ro + 1, slot).translate(0, -slot / 2));
const tube = section.extrude(L);
// Both ends chamfered: the outside narrows over the chamfer length a down to
// about the hole's diameter less the wall, so it leads in.
const lead = Math.min(s * 0.75, a);
const envelope = polygon([[0, 0], [ro - lead, 0], [ro, a], [ro, L - a], [ro - lead, L], [0, L]]).revolve(360);
const pin = tube.intersect(envelope);

const massG = 7.85e-3 * pin.volume;
const [base, perMm] = lookup(PRICE, d, "price");
const size = `${num(d)}×${num(L)}`;
stock({
  standard: "ISO 8752",
  partNumber: `ISO 8752 ${size} St`,
  description: `Slotted spring pin Ø${num(d)} × ${num(L)}, heavy duty, spring steel: drive into a Ø${num(d)} H12 hole`,
  unitPrice: Math.round((base + perMm * L) * 1000) / 1000,
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "slotted spring pins"),
    supplierSearch("Misumi", `spring pin ${num(d)} ${num(L)}`),
    supplierSearch("Bolt Depot", `${num(d)}mm slotted spring pin`),
    supplierSearch("AliExpress", `${num(d)}x${num(L)} spring pin DIN 1481`),
  ],
});
metric("Hole Ø", d, "mm");
metric("Free outside Ø", d1, "mm");
metric("Wall", s, "mm");

connector("End", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Other end", { at: [0, 0, L], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
return pin;
