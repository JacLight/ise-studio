// A lithophane and its lamp base. The picture is lighthouse.png (drawn for
// this example as lighthouse.svg), added with `./ise image add`.
//
// The panel is plastic whose thickness is the picture: thin where it is
// light, thick where it is dark, so with a light behind it the picture
// appears. White PLA, printed standing up (the relief faces the viewer, the
// back is flat). It stands in a slot in the base, its frame hidden in the
// slot, with two tea lights in pockets behind it.
const photo = image("6781e7f423b6dfc93e338a8209a15c64e6fa49c281dd9acce118ac291b86470f");

const t = p.thin + p.range; // full thickness: the darkest pixels and the frame
const panel = relief(photo, { width: p.width, mode: "lithophane", base: p.thin, depth: p.range, border: p.frame, smooth: 0.08 });
const h = panel.bounds.max[1] - panel.bounds.min[1];

// The base: a block with the slot near its front and the lights behind.
const W = p.width + 24;
const D = 64;
const H = 22;
const slotY = 16; // the panel's back face, from the base's front
const gap = 0.15; // each side of the panel in its slot
let base = box(W, D, H).translate(-W / 2, 0, 0);
base = base.subtract(box(p.width + 2 * gap, t + 2 * gap, p.frame + 0.01).translate(-p.width / 2 - gap, slotY - t - gap, H - p.frame));
// Tea lights (Ø38 × 16 mm cups) sit in pockets behind the panel, their wicks level with the slot's bottom.
const light = cylinder(17, 19.4);
base = base.subtract(light.translate(-p.width / 4, slotY + 24, H - 16.5), light.translate(p.width / 4, slotY + 24, H - 16.5));
// A cable-free lamp: a vent under each light keeps the wax cool and the flame fed.
base = base.subtract(cylinder(H, 4).translate(-p.width / 4, slotY + 24, -1), cylinder(H, 4).translate(p.width / 4, slotY + 24, -1));
// The title, cut into the front face.
base = base.engrave(text("LIGHTHOUSE", { size: 5.5, weight: 600, spacing: 1.2, align: "center", valign: "middle" }), { at: [0, 0, H / 2], normal: [0, -1, 0], depth: 0.6 });

// Stand the panel up in its slot: the picture's top to +Z, its relief to the front (−Y).
const standing = panel.rotate(90, 0, 0).translate(0, slotY, H - p.frame + h / 2);

metric("Panel", p.width, "mm wide");
metric("Picture height", h - 2 * p.frame, "mm");
return [
  part("Panel", standing, { material: "PLA (Bambu, white)", process: "3d_print" }),
  part("Base", base, { material: "PLA (Bambu, black)", process: "3d_print" }),
];
