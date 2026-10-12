// UPN: European channels with tapered flanges (U, DIN 1026-1), EN 10365,
// cut to length.
//
// Sources: EN 10365:2017 / DIN 1026-1 tables (as ArcelorMittal's section
// catalogue prints them): depth h, flange width b, web tw, flange tf, root
// radius r1, toe radius r2, and G, the mass per metre at 7850 kg/m³. The
// inside faces of the flanges slope 8 % (h ≤ 300) or 5 % (h > 300); tf is
// the thickness halfway along the flange (8 %: at b/2 from the back of the
// web; 5 %: halfway along the inside, at (b + tw)/2), as DIN 1026-1 measures
// it. The modelled section is within 0.9 % of G. Hot rolled, S235JR to
// EN 10025-2, sold by the metre and sawn to length.
//
// Stands on its end: the back of the web on x = 0, the flanges toward +X,
// the depth along Y centred on y = 0, the length along +Z from z = 0.

const TABLE = { // h, b, tw, tf, r1, r2 (mm), G (kg/m)
  50: [50, 38, 5, 7, 7, 3.5, 5.59],
  65: [65, 42, 5.5, 7.5, 7.5, 4, 7.09],
  80: [80, 45, 6, 8, 8, 4, 8.64],
  100: [100, 50, 6, 8.5, 8.5, 4.5, 10.6],
  120: [120, 55, 7, 9, 9, 4.5, 13.4],
  140: [140, 60, 7, 10, 10, 5, 16],
  160: [160, 65, 7.5, 10.5, 10.5, 5.5, 18.8],
  180: [180, 70, 8, 11, 11, 5.5, 22],
  200: [200, 75, 8.5, 11.5, 11.5, 6, 25.3],
  220: [220, 80, 9, 12.5, 12.5, 6.5, 29.4],
  240: [240, 85, 9.5, 13, 13, 6.5, 33.2],
  260: [260, 90, 10, 14, 14, 7, 37.9],
  280: [280, 95, 10, 15, 15, 7.5, 41.8],
  300: [300, 100, 10, 16, 16, 8, 46.2],
  320: [320, 100, 14, 17.5, 17.5, 8.75, 59.5],
  350: [350, 100, 14, 16, 16, 8, 60.6],
  380: [380, 102, 13.5, 16, 16, 8, 63.1],
  400: [400, 110, 14, 18, 18, 9, 71.8],
};

const [h, b, tw, tf, r1, r2, G] = lookup(TABLE, p.size, "UPN");
const slope = h <= 300 ? 0.08 : 0.05;
const L = p.length;
const { solid } = channelSection({ h, b, tw, tf, r1, r2, slope, tAt: slope > 0.06 ? b / 2 : (b + tw) / 2 }, L);
const designation = `UPN ${p.size}`;
cutToLength({
  designation,
  grade: "S235JR",
  standard: "EN 10365",
  description: `${designation} channel, hot rolled EN 10365`,
  kgPerM: G,
  rate: "beam",
  L,
  sources: steelSources(`${designation} channel`),
});
metric("Depth h", h, "mm");
metric("Width b", b, "mm");
metric("Web tw", tw, "mm");
metric("Flange tf", tf, "mm");
metric("Root radius r1", r1, "mm");
metric("Toe radius r2", r2, "mm");
metric("Flange slope", slope * 100, "%");
return solid;
