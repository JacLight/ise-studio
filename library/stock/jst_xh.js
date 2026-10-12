// JST XH, 2.5 mm pitch wire-to-board connector: B●B-XH-A top-entry
// through-hole headers and XHP-● crimp housings (SXH-001T-P0.6 contacts),
// 2 to 8 circuits.
//
// Source: JST XH catalogue (eXH.pdf) — header B = A + 4.9 mm wide
// (A = (n − 1) × 2.5), 5.75 mm deep, 7.0 mm above the board, 0.64 mm square
// posts with 3.4 mm tails, Ø1.0 mm board holes; the body 2.45 mm beyond the
// end pins and from 2.35 mm in front of the pin row to 3.4 mm behind it (as
// in the KiCad footprint JST_XH_B●B-XH-A). The housing's outline is the
// header's cavity less 0.05 mm all round, 9.8 mm tall; wall, floor and
// keyway sizes are measured from parts, not from the catalogue.

const XH = { pitch: 2.5, pin: 0.64, tail: 3.4, H: 7.0, y0: -2.35, y1: 3.4, side: 2.45, wall: 0.8, floor: 2.4, key: 1.0, Hh: 9.8, hole: 1.0 };
const n = Math.round(p.pins);
const housing = Math.round(p.kind) === 1;
const { parts, massG } = jstConnector(XH, housing ? "housing" : "header", n);
const mpn = housing ? `XHP-${n}` : `B${n}B-XH-A(LF)(SN)`;
stock({
  standard: "JST XH",
  partNumber: housing ? `XHP-${n}` : `B${n}B-XH-A`,
  description: housing
    ? `JST XH crimp housing, ${n} circuits, 2.5 mm pitch (with ${n} × SXH-001T-P0.6 contacts)`
    : `JST XH header, ${n} circuits, 2.5 mm pitch, top entry, through-hole`,
  manufacturer: "JST",
  mpn,
  unitPrice: cents(housing ? 0.06 + n * 0.035 : 0.08 + n * 0.025),
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("Digi-Key", mpn),
    supplierSearch("LCSC", housing ? `XHP-${n}` : `B${n}B-XH-A`),
    supplierSearch("AliExpress", `JST XH 2.54 ${n}pin ${housing ? "housing" : "header"}`),
  ],
});
return parts;
