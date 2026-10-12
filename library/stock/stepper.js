// Hybrid stepper motor body shared by the NEMA frame families (nema14,
// nema23): square end caps of aluminium with the stator stack between them,
// the pilot boss and mounting holes on the front cap, the shaft with its
// D-flat, and the lead exit on the rear cap. (nema17 has its own program.)
//
// Shaft along +Z, the mounting face on z = 0, the body behind it down −Z.
// The stator stack (laminations, rotor, magnet and windings) is one lumped
// part whose density gives the datasheet mass.

/**
 * f: { sq: flange square, ch: caps' corner chamfer leg, chStack: the
 * stack's, cap: front and rear cap thickness, pilot: [Ø, height], pattern:
 * hole square, hole: { tap: M size, depth } | { through: Ø, flange: ear
 * thickness }, shaft: [Ø, length from the face, flat length, flat depth],
 * leads: "socket" | "wires", bolts: through-bolt square, boltHead: Ø,
 * stackMaterial }, L: body length.
 * Returns the parts and declares the connectors and metrics.
 */
function steppers(f, L) {
  const h = f.sq / 2;
  const outline = (ch) => polygon([
    [-h + ch, -h], [h - ch, -h], [h, -h + ch], [h, h - ch],
    [h - ch, h], [-h + ch, h], [-h, h - ch], [-h, -h + ch],
  ]);
  const hp = f.pattern / 2;
  const holes = [[hp, hp], [-hp, hp], [-hp, -hp], [hp, -hp]];

  let front;
  if (f.hole.tap) {
    // Tapped holes straight into the front cap (NEMA 14 / 17).
    front = outline(f.ch).extrude(f.cap).translate(0, 0, -f.cap);
    const d = f.hole.tap;
    for (const [x, y] of holes) {
      front = front.subtract(internalThreadTool({ d, pitch: coarsePitch(d), z0: -f.hole.depth, z1: 0 }).translate(x, y, 0));
      front = front.subtract(cylinder(0.6, d * 0.42, { top: 0.001 }).translate(x, y, -f.hole.depth - 0.6));
    }
  } else {
    // A flange of ears with clearance holes (NEMA 23): the body behind is
    // relieved at the corners so the screws come in from the back.
    const ear = f.hole.flange;
    front = outline(f.ch).extrude(ear).translate(0, 0, -ear)
      .add(outline(f.chStack).extrude(f.cap - ear + 0.01).translate(0, 0, -f.cap));
    for (const [x, y] of holes) front = front.subtract(cylinder(ear + 0.2, f.hole.through / 2).translate(x, y, -ear - 0.1));
  }
  const [pd, ph] = f.pilot;
  const [sd, sl, fl, fd] = f.shaft;
  front = front.add(cylinder(ph, pd / 2)).subtract(cylinder(f.cap + ph + 1, sd / 2 + 0.5).translate(0, 0, -f.cap - 0.5));
  // Rear cap, with the four through-bolt heads' counterbores at the corners.
  const rearOutline = f.hole.tap ? outline(f.ch) : outline(f.chStack);
  let rear = rearOutline.extrude(f.cap).translate(0, 0, -L);
  const bolt = f.bolts / 2;
  for (const [sx, sy] of [[1, 1], [-1, 1], [-1, -1], [1, -1]]) rear = rear.subtract(cylinder(1.5, f.boltHead / 2).translate(sx * bolt, sy * bolt, -L - 0.01));
  const stack = outline(f.chStack + (f.hole.tap ? 1.2 : 0.6)).extrude(L - 2 * f.cap).translate(0, 0, -L + f.cap);
  // Shaft: from inside the front cap out to its length, the D-flat at the end.
  const flat = box(sd + 2, sd, fl + 0.01).translate(-sd / 2 - 1, sd / 2 - fd, sl - fl);
  const shaft = cylinder(sl + f.cap - 1, sd / 2).translate(0, 0, -f.cap + 1).subtract(flat);

  // Leads: a 6-pin JST-PH socket on the rear cap's −Y side, or four wires
  // (black, green, red, blue) out of a moulded strain relief.
  let leadPart;
  if (f.leads === "socket") {
    leadPart = part("Connector socket", box(16, 5.5, 6.5).translate(-8, -h - 5.5 + 0.01, -L + 1)
      .subtract(box(14.6, 4.5, 5.2).translate(-7.3, -h - 5.5 - 0.01, -L + 1.65)), { material: "Nylon 66" });
  } else {
    const relief = cylinder(6, 3.2).rotate(90, 0, 0).translate(0, -h + 0.01, -L + f.cap / 2);
    const wires = [-1.5, -0.5, 0.5, 1.5].map((x) => cylinder(30, 0.55).rotate(90, 0, 0).translate(x * 1.15, -h - 5.9, -L + f.cap / 2));
    leadPart = part("Leads", union(relief, ...wires), { material: "Lead wire and relief (PVC, copper)" });
  }

  metric("Body length", L, "mm");
  metric("Hole pattern", f.pattern, "mm");
  metric("Pilot diameter", pd, "mm");
  metric("Shaft diameter", sd, "mm");
  connector("Shaft", { at: [0, 0, 0], axis: [0, 0, 1], x: [0, 1, 0], part: "Shaft" });
  connector("Pilot", { at: [0, 0, ph], axis: [0, 0, 1], part: "Front cap" });
  connector("Face", { at: [0, 0, 0], axis: [0, 0, 1], part: "Front cap" });
  holes.forEach(([x, y], i) => connector(`Hole ${i + 1}`, { at: [x, y, 0], axis: [0, 0, 1], part: "Front cap" }));
  connector("Rear face", { at: [0, 0, -L], axis: [0, 0, -1], part: "Rear cap" });

  return [
    part("Front cap", front, { material: "Aluminium" }),
    part("Stator stack", stack, { material: f.stackMaterial }),
    part("Rear cap", rear, { material: "Aluminium" }),
    part("Shaft", shaft, { material: "Steel" }),
    leadPart,
  ];
}
