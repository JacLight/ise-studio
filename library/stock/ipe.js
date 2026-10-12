// IPE: European I-beams with parallel flanges, EN 10365, cut to length.
//
// Sources: EN 10365:2017 tables (as ArcelorMittal's and Tata Steel's section
// catalogues print them): depth h, flange width b, web tw, flange tf, root
// radius r, and G, the mass per metre of the nominal section at 7850 kg/m³.
// The modelled section is that outline with true root fillets; its mass is
// within 1 % of G. Hot rolled, S235JR to EN 10025-2, sold by the metre and
// sawn to length.
//
// Stands on its end: the section centred on the origin, the web along Y
// (depth h), the flanges along X, the length along +Z from z = 0.

const TABLE = { // h, b, tw, tf, r (mm), G (kg/m)
  80: [80, 46, 3.8, 5.2, 5, 6],
  100: [100, 55, 4.1, 5.7, 7, 8.1],
  120: [120, 64, 4.4, 6.3, 7, 10.4],
  140: [140, 73, 4.7, 6.9, 7, 12.9],
  160: [160, 82, 5, 7.4, 9, 15.8],
  180: [180, 91, 5.3, 8, 9, 18.8],
  200: [200, 100, 5.6, 8.5, 12, 22.4],
  220: [220, 110, 5.9, 9.2, 12, 26.2],
  240: [240, 120, 6.2, 9.8, 15, 30.7],
  270: [270, 135, 6.6, 10.2, 15, 36.1],
  300: [300, 150, 7.1, 10.7, 15, 42.2],
  330: [330, 160, 7.5, 11.5, 18, 49.1],
  360: [360, 170, 8, 12.7, 18, 57.1],
  400: [400, 180, 8.6, 13.5, 21, 66.3],
  450: [450, 190, 9.4, 14.6, 21, 77.6],
  500: [500, 200, 10.2, 16, 21, 90.7],
  550: [550, 210, 11.1, 17.2, 24, 106],
  600: [600, 220, 12, 19, 24, 122],
};

const [h, b, tw, tf, r, G] = lookup(TABLE, p.size, "IPE");
const L = p.length;
const { solid } = iSection({ h, b, tw, tf, r }, L);
const designation = `IPE ${p.size}`;
cutToLength({
  designation,
  grade: "S235JR",
  standard: "EN 10365",
  description: `${designation} I-beam, hot rolled EN 10365`,
  kgPerM: G,
  rate: "beam",
  L,
  sources: steelSources(`${designation} beam`),
});
metric("Depth h", h, "mm");
metric("Width b", b, "mm");
metric("Web tw", tw, "mm");
metric("Flange tf", tf, "mm");
metric("Root radius r", r, "mm");
return solid;
