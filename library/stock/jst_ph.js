// JST PH, 2.0 mm pitch wire-to-board connector: B●B-PH-K-S top-entry
// through-hole headers and PHR-● crimp housings (SPH-002T-P0.5S contacts),
// 2 to 8 circuits.
//
// Source: JST PH catalogue (ePH.pdf) — header B = A + 3.9 mm wide
// (A = (n − 1) × 2.0), 4.5 mm deep, 6.0 mm above the board, 0.5 mm square
// posts with 3.4 mm tails, Ø0.8 mm board holes; the body 1.95 mm beyond the
// end pins and from 1.7 mm in front of the pin row to 2.8 mm behind it (as
// in the KiCad footprint JST_PH_B●B-PH-K). The housing's outline is the
// header's cavity less 0.05 mm all round, 6.0 mm tall; wall, floor and
// keyway sizes are measured from parts, not from the catalogue.

const PH = { pitch: 2.0, pin: 0.5, tail: 3.4, H: 6.0, y0: -1.7, y1: 2.8, side: 1.95, wall: 0.6, floor: 1.6, key: 0.8, Hh: 6.0, hole: 0.8 };
const n = Math.round(p.pins);
const housing = Math.round(p.kind) === 1;
const { parts, massG } = jstConnector(PH, housing ? "housing" : "header", n);
const mpn = housing ? `PHR-${n}` : `B${n}B-PH-K-S(LF)(SN)`;
stock({
  standard: "JST PH",
  partNumber: housing ? `PHR-${n}` : `B${n}B-PH-K-S`,
  description: housing
    ? `JST PH crimp housing, ${n} circuits, 2.0 mm pitch (with ${n} × SPH-002T-P0.5S contacts)`
    : `JST PH header, ${n} circuits, 2.0 mm pitch, top entry, through-hole`,
  manufacturer: "JST",
  mpn,
  unitPrice: cents(housing ? 0.05 + n * 0.04 : 0.09 + n * 0.025),
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("Digi-Key", mpn),
    supplierSearch("LCSC", housing ? `PHR-${n}` : `B${n}B-PH-K-S`),
    supplierSearch("Adafruit", `JST PH ${n}-pin`),
  ],
});
return parts;
