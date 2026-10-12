// Through-hole LEDs, 3 mm (T-1) and 5 mm (T-1¾), round diffused lens.
//
// Sources: Kingbright WP7113 (5 mm) and WP710A10 (3 mm) datasheets —
// 5 mm: Ø5.0 lens, 8.6 mm from the flange's underside to the dome's top,
// Ø5.8 × 1.0 mm flange with a flat on the cathode side, 0.5 mm square
// leads on 2.54 mm, 25.4 mm min. below the flange (anode the longer);
// 3 mm: Ø3.0 lens, 5.3 mm high, Ø3.8 × 1.0 mm flange, the same leads.
// Inside the lens the cathode's reflector cup (the anvil) and the anode's
// post, as in the real lead frame.
//
// Seated on the board: the flange's underside on z = 0, the lens up +Z,
// leads down −Z. Body centred on the origin; the cathode at x = −1.27, the
// anode at x = +1.27 (the flat faces −X).

const COLOURS = { 0: "red", 1: "green", 2: "yellow", 3: "blue", 4: "white" };
const SIZES = {
  // dia: [lens Ø, height, flange Ø, flange t, flat at, anode lead, cathode lead, price]
  3: [3.0, 5.3, 3.8, 1.0, 1.55, 24, 22.5, 0.03],
  5: [5.0, 8.6, 5.8, 1.0, 2.65, 27, 25.4, 0.04],
};
const dia = Math.round(p.size);
const colour = COLOURS[Math.round(p.colour)];
const [D, H, F, ft, flat, la, lc, base] = lookup(SIZES, dia, "LED size");
const R = D / 2;
// Lens: flange (with the cathode flat), cylinder, hemispherical dome.
const flange = cylinder(ft, F / 2).subtract(box(F, F, ft + 0.2).translate(-flat - F, -F / 2, -0.1));
const lens0 = flange.add(cylinder(H - R - 0.01, R)).add(sphere(R).translate(0, 0, H - R).intersect(cylinder(R + 0.1, R + 0.1).translate(0, 0, H - R)));
// Leads: straight 0.5 mm square through the board, and inside the lens
// the anvil (cathode, a cup under the die) and the post (anode).
// On the 3 mm the leads jog in under the flange to fit inside the lens.
const xi = dia === 5 ? 1.27 : 0.75;
const leadOf = (s, below, top) => {
  const out = squarePin(0.5, -below, 0.6, { tipTop: false }).translate(s * 1.27, 0, 0);
  const jog = xi < 1.27 ? box(1.27 - xi + 0.5, 0.5, 0.5).translate(s > 0 ? xi - 0.25 : -1.27 - 0.25, -0.25, 0.1) : null;
  const up = box(0.5, 0.5, top - 0.1).translate(s * xi - 0.25, -0.25, 0.1);
  return jog ? union(out, jog, up) : out.add(up);
};
const cupTop = 0.62 * H;
const anvil = box(1.25, 0.5, 0.18 * H).translate(-xi - 0.25, -0.25, cupTop - 0.18 * H)
  .subtract(cylinder(1, 0.35).translate(-xi + 0.45, 0, cupTop - 0.25));
const post = box(0.9, 0.5, 0.5).translate(xi - 0.65, -0.25, 0.58 * H - 0.5);
const leads = union(leadOf(-1, lc, cupTop - 0.18 * H + 0.01), leadOf(1, la, 0.58 * H), anvil, post);
const lens = lens0.subtract(leads);
const { parts, massG } = weighed([
  ["Lens", lens, `Epoxy lens (${colour})`],
  ["Leads", leads, "Steel, nickel-plated"],
]);
const pn = `LED ${dia} mm ${colour}`;
stock({
  partNumber: pn,
  description: `${dia} mm through-hole LED, ${colour}, round diffused lens, 2.54 mm lead spacing (typ. Kingbright ${dia === 5 ? "WP7113" : "WP710A10"} series)`,
  manufacturer: "Kingbright",
  unitPrice: cents(base + (colour === "blue" || colour === "white" ? 0.03 : 0)),
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("Digi-Key", `${dia}mm LED ${colour} diffused through hole`),
    supplierSearch("LCSC", `${dia}mm LED ${colour} DIP`),
    supplierSearch("AliExpress", `${dia}mm LED ${colour} diffused`),
  ],
});
metric("Lens diameter", D, "mm");
metric("Height", H, "mm");
metric("Panel hole", D + 0.2, "mm");

connector("Board", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
connector("Lens tip", { at: [0, 0, H], axis: [0, 0, 1], x: [1, 0, 0] });
connector("Cathode", { at: [-1.27, 0, 0], axis: [0, 0, -1] });
connector("Anode", { at: [1.27, 0, 0], axis: [0, 0, -1] });
return parts;
