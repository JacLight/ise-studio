// Self-adhesive bumper (bump-on foot), clear polyurethane on an acrylic
// adhesive: dome or cylinder.
//
// Source: the makers' standard sizes (3M Bumpon SJ-series and the generic
// equivalents sold by the sheet): diameter D and height H; a cylinder's
// top edge rounded a quarter of its height.
//
// The adhesive face sits on z = 0 against the part it is stuck under, the
// bumper hanging down −Z to the floor at z = −H.
// Mass: this nominal geometry at 1.1 g/cm³ (polyurethane).

const D = p.D;
const H = p.H;
const R = D / 2;
const dome = p.kind < 1.5;
let bumper;
if (dome) {
  // A spherical cap of base radius R and height H.
  const Rs = (R * R + H * H) / (2 * H);
  bumper = cylinder(H, R).intersect(sphere(Rs).translate(0, 0, H - Rs));
} else {
  const re = 0.25 * H;
  bumper = union(
    cylinder(H - re + 0.01, R),
    cylinder(re, R - re).translate(0, 0, H - re),
    circle(re).translate(R - re, H - re).revolve(360),
  );
}
bumper = bumper.mirror(0, 0, 1);

const massG = 1.1e-3 * bumper.volume;
const shape = dome ? "dome" : "cylinder";
const size = `Ø${num(D)}×${num(H)}`;
stock({
  partNumber: `Bumper ${shape} ${size} PU clear`,
  description: `Self-adhesive bumper foot, ${shape} ${size} mm, clear polyurethane`,
  unitPrice: Math.round((0.02 + 0.00025 * D * D * H) * 1000) / 1000,
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "bumpers"),
    supplierSearch("Misumi", `rubber bumper adhesive ${num(D)}x${num(H)}`),
    supplierSearch("Amazon", `clear adhesive bumper pads ${num(D)}mm x ${num(H)}mm ${shape}`),
    supplierSearch("AliExpress", `self adhesive rubber feet ${num(D)}x${num(H)}mm ${dome ? "hemispherical" : "cylindrical"}`),
  ],
});
metric("Diameter", D, "mm");
metric("Height", H, "mm");

// The adhesive face (the part comes from +Z) and where it meets the floor.
connector("Mount face", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("Floor contact", { at: [0, 0, -H], axis: [0, 0, -1] });
connector("Axis", { at: [0, 0, -H], axis: [0, 0, 1] });
return bumper;
