// HE A (IPB light, HEA): European wide-flange H-beams, EN 10365, cut to length.
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
  100: [96, 100, 5, 8, 12, 16.7],
  120: [114, 120, 5, 8, 12, 19.9],
  140: [133, 140, 5.5, 8.5, 12, 24.7],
  160: [152, 160, 6, 9, 15, 30.4],
  180: [171, 180, 6, 9.5, 15, 35.5],
  200: [190, 200, 6.5, 10, 18, 42.3],
  220: [210, 220, 7, 11, 18, 50.5],
  240: [230, 240, 7.5, 12, 21, 60.3],
  260: [250, 260, 7.5, 12.5, 24, 68.2],
  280: [270, 280, 8, 13, 24, 76.4],
  300: [290, 300, 8.5, 14, 27, 88.3],
  320: [310, 300, 9, 15.5, 27, 97.6],
  340: [330, 300, 9.5, 16.5, 27, 105],
  360: [350, 300, 10, 17.5, 27, 112],
  400: [390, 300, 11, 19, 27, 125],
  450: [440, 300, 11.5, 21, 27, 140],
  500: [490, 300, 12, 23, 27, 155],
  550: [540, 300, 12.5, 24, 27, 166],
  600: [590, 300, 13, 25, 27, 178],
};

const [h, b, tw, tf, r, G] = lookup(TABLE, p.size, "HE A");
const L = p.length;
const { solid } = iSection({ h, b, tw, tf, r }, L);
const designation = `HE ${p.size} A`;
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
