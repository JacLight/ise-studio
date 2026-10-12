// HE B (IPB, HEB): European wide-flange H-beams, EN 10365, cut to length.
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
  100: [100, 100, 6, 10, 12, 20.4],
  120: [120, 120, 6.5, 11, 12, 26.7],
  140: [140, 140, 7, 12, 12, 33.7],
  160: [160, 160, 8, 13, 15, 42.6],
  180: [180, 180, 8.5, 14, 15, 51.2],
  200: [200, 200, 9, 15, 18, 61.3],
  220: [220, 220, 9.5, 16, 18, 71.5],
  240: [240, 240, 10, 17, 21, 83.2],
  260: [260, 260, 10, 17.5, 24, 93],
  280: [280, 280, 10.5, 18, 24, 103],
  300: [300, 300, 11, 19, 27, 117],
  320: [320, 300, 11.5, 20.5, 27, 127],
  340: [340, 300, 12, 21.5, 27, 134],
  360: [360, 300, 12.5, 22.5, 27, 142],
  400: [400, 300, 13.5, 24, 27, 155],
  450: [450, 300, 14, 26, 27, 171],
  500: [500, 300, 14.5, 28, 27, 187],
  550: [550, 300, 15, 29, 27, 199],
  600: [600, 300, 15.5, 30, 27, 212],
};

const [h, b, tw, tf, r, G] = lookup(TABLE, p.size, "HE B");
const L = p.length;
const { solid } = iSection({ h, b, tw, tf, r }, L);
const designation = `HE ${p.size} B`;
cutToLength({
  designation,
  grade: "S235JR",
  standard: "EN 10365",
  description: `${designation} wide-flange beam, hot rolled EN 10365`,
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
