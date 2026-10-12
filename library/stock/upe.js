// UPE: European channels with parallel flanges, EN 10365, cut to length.
//
// Sources: EN 10365:2017 tables (as ArcelorMittal's section catalogue
// prints them): depth h, flange width b, web tw, flange tf, root radius r,
// and G, the mass per metre at 7850 kg/m³. Flange toes are square. The
// modelled section is within 1 % of G. Hot rolled, S235JR to EN 10025-2,
// sold by the metre and sawn to length.
//
// Stands on its end: the back of the web on x = 0, the flanges toward +X,
// the depth along Y centred on y = 0, the length along +Z from z = 0.

const TABLE = { // h, b, tw, tf, r (mm), G (kg/m)
  80: [80, 50, 4, 7, 10, 7.9],
  100: [100, 55, 4.5, 7.5, 10, 9.82],
  120: [120, 60, 5, 8, 12, 12.1],
  140: [140, 65, 5, 9, 12, 14.5],
  160: [160, 70, 5.5, 9.5, 12, 17],
  180: [180, 75, 5.5, 10.5, 12, 19.7],
  200: [200, 80, 6, 11, 13, 22.8],
  220: [220, 85, 6.5, 12, 13, 26.6],
  240: [240, 90, 7, 12.5, 15, 30.2],
  270: [270, 95, 7.5, 13.5, 15, 35.2],
  300: [300, 100, 9.5, 15, 15, 44.4],
  330: [330, 105, 11, 16, 18, 53.2],
  360: [360, 110, 12, 17, 18, 61.2],
  400: [400, 115, 13.5, 18, 18, 72.2],
};

const [h, b, tw, tf, r, G] = lookup(TABLE, p.size, "UPE");
const L = p.length;
const { solid } = channelSection({ h, b, tw, tf, r1: r }, L);
const designation = `UPE ${p.size}`;
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
metric("Root radius r", r, "mm");
return solid;
