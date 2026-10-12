// A bas-relief wall plaque carved from a picture: brightness becomes height,
// so the moonlit pines and the near bank stand proudest and the night sky
// sits deepest. The picture is valley.jpg (drawn for this example as
// valley.svg), added to the project with `./forma image add valley.jpg`;
// programs refer to it by its content hash.
//
// Printed face up in one piece: a raised frame all round, the relief inside
// it, two keyhole slots in the back for screws in the wall.
const photo = image("76ec604bd3ea568023abefcc0b7fe4a2026da8259fe744ede865ef153056c711");

const plaque = relief(photo, {
  width: p.width,
  depth: p.depth, // the brightest pine tip stands this far above the darkest sky
  base: p.base, // solid back under the deepest point
  smooth: p.smooth, // soften the picture's pixel steps (mm)
  gamma: 1.3, // deepen the darks a little so the sky reads as space
  border: p.frame,
});

// Keyhole slots in the back (z = 0 is against the wall): the screw head goes
// in the round entry, slides up, and the shank holds in the narrow slot
// while the head sits in the channel behind it. Blind: the face stays whole.
const { min, max } = plaque.bounds;
const keyhole = (x) => {
  const y = max[1] - 24;
  const entry = cylinder(2.2 + 0.5, 4.6).translate(x, y, -0.5);
  const slot = box(4.4, 9, 1.5).translate(x - 2.2, y, -0.5); // the shank, at the wall
  const channel = box(9.2, 9, 1.2).translate(x - 4.6, y, 1.0); // the head, behind it
  return entry.add(slot, channel, cylinder(1.2, 4.6).translate(x, y + 9, 1.0));
};
metric("Width", max[0] - min[0], "mm");
metric("Height", max[1] - min[1], "mm");
return [part("Plaque", plaque.subtract(keyhole(min[0] + 40), keyhole(max[0] - 40)), { material: "PLA (Bambu, white)", process: "3d_print" })];
