// T-slot nut for aluminium extrusion: sliding (slides in from the end),
// hammer (drops in through the slot and turns 90° to lock) or roll-in with
// a spring ball (drops in, rolls under the lips, the ball holds it), zinc
// plated steel, tapped through.
//
// Sources: the 20 series 6 mm and 30 / 40 series 8 mm slot nuts sold for
// these profiles (Misumi HNTT / HNTAJ pattern, the generic "M5 hammer nut
// 2020" and "roll-in T-nut with spring ball"): a neck that rides in the slot
// opening (0.4 mm narrower), a body under the lips that follows the slot
// cavity with 0.3 mm clearance to half-way down its tapered bottom, lengths
// along the slot 10 / 14 / 16 mm; a hammer nut is as long as the neck is
// wide along the slot, two opposite corners rounded so it turns one way.
// The slot shapes are those of tslot_20 / tslot_30 / tslot_40 (tslot.js).
// Mass: the modelled steel.
//
// Placed in its slot: the extrusion's face on z = 0 with the slot running
// along Y, the nut's top bearing against the lips' underside at z = −lip,
// the neck up to z = −0.3; the thread along Z. Join "Slot face" to an
// extrusion's "Slot …" connector (the nut drops into that slot).

const series = p.series;
const s = lookup(TSLOT_SERIES, series, "T-slot series");
const d = p.d;
const type = Math.round(p.type); // 0 sliding, 1 hammer, 2 roll-in
const TYPES = ["sliding", "hammer", "roll-in"];
if (TYPES[type] === undefined) throw new Error("type is 0 (sliding), 1 (hammer) or 2 (roll-in)");
const nh = s.slot / 2 - 0.2; // neck half width
const hw0 = s.tHalf - 0.3; // body half width under the lips
const k = 0.5; // how far down the tapered bottom the body goes
const zb = -(s.tDepth + k * (s.depth - s.tDepth));
const hwb = s.tHalf - k * (s.tHalf - s.tipHalf) - 0.3;
const zTop = -0.3;
if (d / 2 + 0.3 > nh) throw new Error(`an M${d} thread does not fit a ${series} series nut's ${2 * nh} mm neck`);

// The section across the slot, in (x, z).
const sectionPts = [[-nh, zTop], [nh, zTop], [nh, -s.lip], [hw0, -s.lip], [hw0, -s.tDepth], [hwb, zb], [-hwb, zb], [-hw0, -s.tDepth], [-hw0, -s.lip], [-nh, -s.lip]];
let section = polygon(sectionPts);
const lengths = { 20: 10, 30: 14, 40: 16 };
const ln = type === 1 ? 2 * nh : lengths[series];
if (type === 2) {
  // Roll-in: the −X bottom corner rounded so it rolls under the lip.
  const hb = -s.lip - zb;
  const cx = -hw0 + hb;
  const keep = circle(hb).translate(cx, -s.lip).add(rect(2 * hw0, 20).translate(cx, zb - 1), rect(4 * hw0, 20).translate(-2 * hw0, -s.lip));
  section = section.intersect(keep);
}
// Extrude along the slot (Y): profile (x, y) → (x, z) with the length on Y.
let nut = section.extrude(ln).rotate(90, 0, 0).translate(0, ln / 2, 0);
if (type === 1) {
  // Hammer: two opposite corners rounded in plan.
  const plan = circle(hw0).add(rect(hw0 + 1, ln / 2 + 1), rect(hw0 + 1, ln / 2 + 1).translate(-hw0 - 1, -ln / 2 - 1));
  nut = nut.intersect(plan.extrude(-zb + 1).translate(0, 0, zb - 0.5));
}
const pitch = coarsePitch(d);
nut = nut.subtract(internalThreadTool({ d, pitch, z0: zb, z1: zTop }));

const parts = [];
if (type === 2) {
  const rb = series === 20 ? 0.75 : 1.0;
  const xb = (nh + hw0) / 2;
  nut = nut.subtract(cylinder(2 * rb + 0.2, rb + 0.02).translate(xb, 0, -s.lip - 2 * rb - 0.1));
  parts.push(part("Spring ball", sphere(rb).translate(xb, 0, -s.lip - rb - 0.02), { material: "Bearing steel 52100" }));
}
parts.unshift(part("Nut", nut, { material: "Steel, zinc plated" }));

const massG = parts.reduce((m, x) => m + x.solid.volume * 7.85e-3, 0);
const slot = series === 20 ? 6 : 8;
stock({
  partNumber: `TNUT-${series}-${TYPES[type].toUpperCase()}-M${num(d)}`,
  description: `T-slot nut, ${TYPES[type]}${type === 2 ? " with spring ball" : ""}, M${num(d)}, for ${series} series ${slot} mm slot, zinc-plated steel`,
  unitPrice: cents({ 20: 0.07, 30: 0.12, 40: 0.16 }[series] + (type === 2 ? 0.04 : 0)),
  currency: "USD",
  pack: 50,
  massG: Math.round(massG * 100) / 100,
  sources: [
    supplierSearch("Misumi", `${TYPES[type]} nut ${series} series ${slot}mm slot M${num(d)}`),
    supplierSearch("McMaster-Carr", "t-slotted framing nuts"),
    supplierSearch("AliExpress", `M${num(d)} ${type === 1 ? "hammer" : type === 2 ? "roll in" : "sliding"} t nut ${series}${series} ${slot}mm slot`),
    supplierSearch("Amazon", `M${num(d)} t nut ${series}${series} ${TYPES[type]}`),
  ],
});
metric("Tapped thread", d, "mm");
metric("Tapped depth", zTop - zb, "mm");
metric("Length along slot", ln, "mm");

connector("Slot face", { at: [0, 0, 0], axis: [0, 0, -1], x: [0, 1, 0] });
connector("Thread", { at: [0, 0, zTop], axis: [0, 0, 1] });
connector("Lip seat", { at: [0, 0, -s.lip], axis: [0, 0, 1] });
return parts;
