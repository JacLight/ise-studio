// Hot-rolled equal-leg T sections (T 30 to T 140), EN 10055, cut to length.
//
// Sources: EN 10055:1995 table (as DIN 1024 / merchant-bar catalogues print
// it): flange width b = height h, flange and stem thickness t, root radius
// r1 = t, flange-toe radius r2 = t/2, stem-toe radius r3 = t/4, and G, the
// mass per metre at 7850 kg/m³. EN 10055 rolls the flange and stem with a
// slight taper (2 %); this section is drawn parallel at the nominal t, which
// keeps the mass within 2 % of G. S235JR to EN 10025-2, sold by the metre
// and sawn to length.
//
// Stands on its end: the flange's top face on y = h, the stem down to y = 0,
// centred on x = 0, the length along +Z from z = 0.

const TABLE = { // t (mm), G (kg/m)
  30: [4, 1.77],
  35: [4.5, 2.33],
  40: [5, 2.96],
  45: [5.5, 3.67],
  50: [6, 4.44],
  60: [7, 6.23],
  70: [8, 8.32],
  80: [9, 10.7],
  100: [11, 16.4],
  120: [13, 23.2],
  140: [15, 31.3],
};

const h = p.h;
const b = h;
const [t, G] = lookup(TABLE, h, "EN 10055 T");
const L = p.length;
const outline = roundedOutline([
  [-t / 2, 0, t / 4], [t / 2, 0, t / 4], [t / 2, h - t, t], [b / 2, h - t, t / 2], [b / 2, h],
  [-b / 2, h], [-b / 2, h - t, t / 2], [-t / 2, h - t, t],
]);
const { solid, props } = member(outline, L);
memberConnectors(L, 0, props.cy);
faceConnector("Flange top", 0, h, L, [0, 1]);
faceConnector("Flange underside +X", (b / 2 + t / 2) / 2, h - t, L, [0, -1]);
faceConnector("Flange underside -X", -(b / 2 + t / 2) / 2, h - t, L, [0, -1]);
faceConnector("Stem +X", t / 2, (h - t) / 2, L, [1, 0]);
faceConnector("Stem -X", -t / 2, (h - t) / 2, L, [-1, 0]);
faceConnector("Stem toe", 0, 0, L, [0, -1]);
const designation = `T ${num(h)}`;
cutToLength({
  designation,
  grade: "S235JR",
  standard: "EN 10055",
  description: `${designation} (${num(h)}×${num(h)}×${num(t)}) tee, hot rolled EN 10055`,
  kgPerM: G,
  rate: "bar",
  L,
  sources: steelSources(`T section ${num(h)}x${num(h)}x${num(t)} steel`),
});
metric("Height h", h, "mm");
metric("Thickness t", t, "mm");
metric("Centroid from stem toe", Math.round(props.cy * 100) / 100, "mm");
return solid;
