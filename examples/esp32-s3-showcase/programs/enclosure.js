// ISE S3 Handheld: the case around the ESP32-S3 mainboard, in the board's
// coordinates (board lower-left corner at the origin, its bottom face on z = 0,
// top face z = 1.6). Two printed shells meet on z = 0; four M2 screws go up
// through the back shell and the board's mounting holes into heat-set inserts
// in the front shell's bosses, clamping the board between the bosses.
//
// Everything that sits in the case is placed from the board layout
// (examples/esp32-s3-showcase): buttons, light pipes, the microphone port,
// USB-C, Qwiic, the power slide switch, the microSD slot, the BME280 vents.
// `explode` (mm) opens the product for an exploded view: the front shell, display, lens, caps
// and light pipes lift off the board, which stays in the back shell with the cell and speaker. The product stands
// on the ground: everything is lifted by LIFT so the back shell's bottom is on z = 0 (the
// board's 3D part sits at z = LIFT too).

const E = p.explode ?? 0;
const LIFT = 13.0;
const T = 1.6; // board thickness
const BOARD = { w: 50, h: 72, r: 3 };
const GAP = 0.5, WALL = 2.0, TOP = 1.2, FLOOR = 1.4;
const X0 = -GAP - WALL, Y0 = -GAP - WALL, X1 = BOARD.w + GAP + WALL, Y1 = BOARD.h + GAP + WALL;
const RO = BOARD.r + GAP + WALL;
const ZF = 8.6; // front shell top face
const ZB = -13.0; // back shell bottom face
const CEIL = ZF - TOP;
const HOLES = [[3.5, 3.5], [46.5, 3.5], [3.5, 62.2], [46.5, 62.2]];
const BUTTONS = [[8.5, 20.5, "A"], [25, 20.5, "OK"], [41.5, 20.5, "B"]];
const PLUNGER_TOP = T + 4.3; // C&K PTS645 Sx43
const DISPLAY = { cx: 25, cy: 43.86, w: 31.52, h: 33.72, t: 2.1, aa: 27.72, aaDy: 1.5 }; // 1.54" 240×240 IPS module
const LENS = { w: 33.6, h: 36.0, t: 0.7 };
const INSERT = { hole: 3.2, depth: 5 }; // Ruthex M2×4: printed hole Ø3.2, 5 deep
const SCREW = { clear: 2.4, cbore: 4.2, seat: -7.0 }; // M2×12 ISO 4762, head Ø3.8

/** A box from corner to corner with its upright edges rounded. */
function rbox(x0, y0, x1, y1, z0, z1, r) {
  const b = box(x1 - x0, y1 - y0, z1 - z0).translate(x0, y0, z0);
  return r > 0 ? b.fillet(Math.min(r, (x1 - x0) / 2 - 0.01, (y1 - y0) / 2 - 0.01), { parallel: "z" }) : b;
}
const cyl = (x, y, z0, z1, d) => cylinder(z1 - z0, d / 2).translate(x, y, z0);
/** Edges lying in the plane z (a shell's rim). */
const rim = (z) => ({ within: { min: [X0 - 1, Y0 - 1, z - 0.01], max: [X1 + 1, Y1 + 1, z + 0.01] } });
/** A slot through a wall along x (toward ±x) or y, rounded on its long edges. */
function slotX(x0, x1, y0, y1, z0, z1, r) {
  const b = box(x1 - x0, y1 - y0, z1 - z0).translate(x0, y0, z0);
  return r > 0 ? b.fillet(r, { parallel: "x" }) : b;
}
function slotY(x0, x1, y0, y1, z0, z1, r) {
  const b = box(x1 - x0, y1 - y0, z1 - z0).translate(x0, y0, z0);
  return r > 0 ? b.fillet(r, { parallel: "y" }) : b;
}

// ---- Front shell -------------------------------------------------------------------
let front = rbox(X0, Y0, X1, Y1, 0, ZF, RO).fillet(1.6, rim(ZF));
front = front.subtract(rbox(X0 + WALL, Y0 + WALL, X1 - WALL, Y1 - WALL, -1, CEIL, RO - WALL));
// Display: the cover lens sits in a recess, the active area shows through the window.
const winY = DISPLAY.cy + DISPLAY.aaDy;
front = front.subtract(
  rbox(DISPLAY.cx - LENS.w / 2 - 0.15, DISPLAY.cy - LENS.h / 2 - 0.15, DISPLAY.cx + LENS.w / 2 + 0.15, DISPLAY.cy + LENS.h / 2 + 0.15, ZF - LENS.t, ZF + 1, 2.15),
  rbox(DISPLAY.cx - DISPLAY.aa / 2 - 0.3, winY - DISPLAY.aa / 2 - 0.3, DISPLAY.cx + DISPLAY.aa / 2 + 0.3, winY + DISPLAY.aa / 2 + 0.3, CEIL - 1, ZF, 1.0),
);
// Display locating frame on the ceiling, open where the FPC folds round.
{
  const g = 0.2, t = 0.8, z0 = CEIL - 1.6;
  const [x0, x1, y0, y1] = [DISPLAY.cx - DISPLAY.w / 2 - g, DISPLAY.cx + DISPLAY.w / 2 + g, DISPLAY.cy - DISPLAY.h / 2 - g, DISPLAY.cy + DISPLAY.h / 2 + g];
  const frame = union(
    box(t, y1 - y0 + 2 * t, CEIL - z0).translate(x0 - t, y0 - t, z0),
    box(t, y1 - y0 + 2 * t, CEIL - z0).translate(x1, y0 - t, z0),
    box(x1 - x0, t, CEIL - z0).translate(x0, y1, z0),
    box(7.5, t, CEIL - z0).translate(x0, y0 - t, z0),
    box(7.5, t, CEIL - z0).translate(x1 - 7.5, y0 - t, z0),
  );
  front = front.add(frame);
}
// Buttons, light pipes, microphone and charge-LED holes in the top.
for (const [x, y] of BUTTONS) front = front.subtract(cyl(x, y, CEIL - 1, ZF + 1, 6.6));
front = front.subtract(cyl(8.0, 63.5, CEIL - 1, ZF + 1, 3.2), cyl(41.5, 63.6, CEIL - 1, ZF + 1, 2.2), cyl(36.0, 15.29, CEIL - 1, ZF + 1, 1.0));
front = front.subtract(cyl(9.0, 3.0, CEIL - 1, ZF + 1, 1.4), cyl(12.6, 3.0, CEIL - 1, ZF + 1, 1.4));
// USB-C (bottom wall), Qwiic (right wall), power slide switch and BME280 vents (left wall).
front = front.subtract(slotY(20.1, 29.9, Y0 - 1, 0.5, T - 0.4, T + 3.6, 1.5));
front = front.subtract(slotX(BOARD.w - 0.5, X1 + 1, 40.1, 46.9, T - 0.3, T + 3.3, 0.8));
front = front.subtract(slotX(X0 - 1, 0.5, 42.0, 46.6, T + 0.1, T + 1.8, 0.5));
for (const y of [27.6, 29.6, 31.6, 33.6]) front = front.subtract(slotX(X0 - 1, 0.5, y - 0.5, y + 0.5, T + 0.6, T + 4.6, 0.45));
// Bosses with heat-set inserts: they press the board onto the back shell's bosses.
for (const [x, y] of HOLES) front = front.add(cyl(x, y, T, CEIL + 0.01, 5.2)).subtract(cyl(x, y, T - 0.1, T + INSERT.depth, INSERT.hole));
// Moulded mark on the top: the name under the screen.
front = front.subtract(text("Iṣẹ́ Studio", { size: 2.4, weight: 700, align: "center", valign: "middle" }).extrude(0.4).translate(25, 66.4, ZF - 0.4));

// ---- Back shell --------------------------------------------------------------------
let back = rbox(X0, Y0, X1, Y1, ZB, 0, RO).fillet(2.0, rim(ZB));
back = back.subtract(rbox(X0 + WALL, Y0 + WALL, X1 - WALL, Y1 - WALL, ZB + FLOOR, 1, RO - WALL));
// Screw bosses, counterbored from outside for the M2 heads.
for (const [x, y] of HOLES) {
  back = back.add(cyl(x, y, ZB + FLOOR - 0.01, 0, 5.6));
  back = back.subtract(cyl(x, y, ZB - 1, 1, SCREW.clear), cyl(x, y, ZB - 1, SCREW.seat, SCREW.cbore));
}
// Battery (503040, 5 × 30 × 40) held in a low frame; speaker ring; their grille.
const BAT = { x0: 3, y0: 11, x1: 33, y1: 51, z0: ZB + FLOOR, t: 5 };
{
  const g = 0.4, t = 1.0, h = 2.0, z = ZB + FLOOR - 0.01;
  const corner = (x, y, sx, sy) => union(box(6, t, h).translate(sx > 0 ? x : x - 6, sy > 0 ? y - t : y, z), box(t, 6, h).translate(sx > 0 ? x - t : x, sy > 0 ? y : y - 6, z));
  back = back.add(corner(BAT.x0 - g, BAT.y0 - g, 1, 1), corner(BAT.x1 + g, BAT.y0 - g, -1, 1), corner(BAT.x0 - g, BAT.y1 + g, 1, -1), corner(BAT.x1 + g, BAT.y1 + g, -1, -1));
}
const SPK = { x: 42.5, y: 21.0, d: 13, t: 3.5 }; // 13 mm micro speaker, 8 Ω
back = back.add(cyl(SPK.x, SPK.y, ZB + FLOOR - 0.01, ZB + FLOOR + 1.5, SPK.d + 2.4).subtract(cyl(SPK.x, SPK.y, ZB, ZB + FLOOR + 2, SPK.d + 0.6)));
for (let i = -3; i <= 3; i++) for (let j = -3; j <= 3; j++) {
  const x = SPK.x + 1.8 * i + (j % 2 ? 0.9 : 0), y = SPK.y + 1.56 * j;
  if (Math.hypot(x - SPK.x, y - SPK.y) < 5.6) back = back.subtract(cyl(x, y, ZB - 1, ZB + FLOOR + 0.5, 1.0));
}
// microSD slot (right wall, under the board), reset and boot pinholes with guide tubes.
back = back.subtract(slotX(BOARD.w - 0.5, X1 + 1, 27.6, 40.4, -2.4, 1, 0.6));
for (const [x, y] of [[9, 54], [41, 49]]) back = back.add(cyl(x, y, ZB + FLOOR - 0.01, -3.2, 3.4)).subtract(cyl(x, y, ZB - 1, 0, 1.6));
back = back.subtract(text("Iṣẹ́ S3", { size: 4, weight: 700, align: "center", valign: "middle" }).extrude(0.4).mirror(1, 0, 0).translate(18, 58, ZB - 0.01));

// ---- Small printed and moulded parts ------------------------------------------------
const caps = BUTTONS.map(([x, y, n]) => {
  const c = cyl(x, y, PLUNGER_TOP + 0.15, PLUNGER_TOP + 0.95, 8.4).add(cyl(x, y, PLUNGER_TOP + 0.9, ZF + 1.0, 6.2)).fillet(0.6, { within: { min: [x - 4, y - 4, ZF + 0.99], max: [x + 4, y + 4, ZF + 1.01] } });
  return [n, c];
});
// Status pipe: a Ø3.4 collar under the top stops it (a wider flange would meet the display frame and the boss).
const pipe1 = cyl(8.0, 63.5, T + 0.95, ZF, 3.0).add(cyl(8.0, 63.5, CEIL - 0.8, CEIL - 0.05, 3.4));
const pipe2 = cyl(41.5, 63.6, T + 0.8, ZF, 2.0).add(cyl(41.5, 63.6, CEIL - 0.8, CEIL - 0.05, 3.4));
// The cover lens: one 0.7 mm acrylic plate, screen-printed black on its back round the window;
// drawn as its two looks (the printed border and the clear window), which together are the plate.
const lensPlate = rbox(DISPLAY.cx - LENS.w / 2, DISPLAY.cy - LENS.h / 2, DISPLAY.cx + LENS.w / 2, DISPLAY.cy + LENS.h / 2, ZF - LENS.t + 0.02, ZF - 0.02, 2.0);
const lensWindow = rbox(DISPLAY.cx - DISPLAY.aa / 2, winY - DISPLAY.aa / 2, DISPLAY.cx + DISPLAY.aa / 2, winY + DISPLAY.aa / 2, ZF - LENS.t + 0.02, ZF - 0.02, 0.6);
const lens = lensWindow;
const border = lensPlate.subtract(lensWindow);

// ---- Bought parts, modelled from their drawings ------------------------------------
const dz = CEIL - DISPLAY.t; // display back face
const lcd = {
  backlight: rbox(DISPLAY.cx - DISPLAY.w / 2, DISPLAY.cy - DISPLAY.h / 2, DISPLAY.cx + DISPLAY.w / 2, DISPLAY.cy + DISPLAY.h / 2, dz, dz + 1.1, 0.3),
  // The glass, its top 0.02 mm split into the border and the active area (one sheet, two looks).
  glass: box(DISPLAY.w, DISPLAY.h, DISPLAY.t - 1.1 - 0.02).translate(DISPLAY.cx - DISPLAY.w / 2, DISPLAY.cy - DISPLAY.h / 2, dz + 1.1)
    .add(box(DISPLAY.w, DISPLAY.h, 0.02).translate(DISPLAY.cx - DISPLAY.w / 2, DISPLAY.cy - DISPLAY.h / 2, CEIL - 0.02).subtract(box(DISPLAY.aa, DISPLAY.aa, 1).translate(DISPLAY.cx - DISPLAY.aa / 2, winY - DISPLAY.aa / 2, CEIL - 0.5))),
  screen: box(DISPLAY.aa, DISPLAY.aa, 0.02).translate(DISPLAY.cx - DISPLAY.aa / 2, winY - DISPLAY.aa / 2, CEIL - 0.02),
  // The FPC leaves the bottom edge, folds down and runs into the FH12 connector.
  // Out of the bottom edge under the glass, down past the board's FPC connector, into its mouth.
  fpc: union(
    box(6.6, 1.0, 0.12).translate(25 - 3.3, DISPLAY.cy - DISPLAY.h / 2 - 1.0, dz + 0.3),
    box(6.6, 0.12, dz + 0.42 - (T + 0.9)).translate(25 - 3.3, DISPLAY.cy - DISPLAY.h / 2 - 1.0, T + 0.9),
    box(6.6, 0.4, 0.12).translate(25 - 3.3, DISPLAY.cy - DISPLAY.h / 2 - 1.0, T + 0.9),
  ),
};
const cell = rbox(BAT.x0, BAT.y0, BAT.x1, BAT.y1, BAT.z0 + 0.05, BAT.z0 + BAT.t, 1.0).fillet(0.8, { within: { min: [BAT.x0 - 1, BAT.y0 - 1, BAT.z0 + BAT.t - 0.01], max: [BAT.x1 + 1, BAT.y1 + 1, BAT.z0 + BAT.t + 0.01] } });
const cellLabel = box(26, 20, 0.05).translate(BAT.x0 + 2, BAT.y0 + 14, BAT.z0 + BAT.t);
const spkZ = ZB + FLOOR + 0.05;
const speaker = {
  frame: cyl(SPK.x, SPK.y, spkZ, spkZ + SPK.t, SPK.d).subtract(cyl(SPK.x, SPK.y, spkZ - 1, spkZ + 0.6, SPK.d - 1.6)),
  cone: cyl(SPK.x, SPK.y, spkZ + 0.25, spkZ + 0.5, SPK.d - 1.6),
};

// ---- Exploded positions ------------------------------------------------------------
const up = (s, k) => s.translate(0, 0, LIFT + E * k);
const out = [
  part("Case/Front shell", up(front, 1.0), { material: "PETG charcoal", process: "3d_print" }),
  part("Case/Back shell", up(back, 0), { material: "PETG charcoal", process: "3d_print" }),
  ...caps.map(([n, c]) => part(`Case/Button cap ${n}`, up(c, 1.45), { material: "PETG orange", process: "3d_print" })),
  part("Case/Light pipe (status LED)", up(pipe1, 1.45), { material: "PC clear", process: "3d_print" }),
  part("Case/Light pipe (light sensor)", up(pipe2, 1.45), { material: "PC clear", process: "3d_print" }),
  part("Case/Cover lens/Window", up(lens, 1.9), { material: "Acrylic clear", process: "laser" }),
  part("Case/Cover lens/Printed border", up(border, 1.9), { material: "Lens black", process: "laser" }),
  part("Display/Backlight", up(lcd.backlight, 0.55), { material: "Display white", process: "buy" }),
  part("Display/Glass", up(lcd.glass, 0.55), { material: "Display glass", process: "buy" }),
  part("Display/Active area", up(lcd.screen, 0.55), { material: "Screen black", process: "buy" }),
  part("Display/FPC", up(lcd.fpc, 0.55), { material: "Polyimide", process: "buy" }),
  part("Battery/Cell", up(cell, 0), { material: "Pouch silver", process: "buy" }),
  part("Battery/Label", up(cellLabel, 0), { material: "Label blue", process: "buy" }),
  part("Speaker/Frame", up(speaker.frame, 0), { material: "Speaker steel", process: "buy" }),
  part("Speaker/Cone", up(speaker.cone, 0), { material: "Screen black", process: "buy" }),
];
metric("Case width", X1 - X0, "mm");
metric("Case length", Y1 - Y0, "mm");
metric("Case thickness", ZF - ZB, "mm");
return out;
