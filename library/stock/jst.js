// JST wire-to-board connectors (XH 2.5 mm, PH 2.0 mm pitch): the top-entry
// through-hole header and the crimp housing that plugs into it, shared by
// jst_xh and jst_ph.
//
// The header stands on the board (z = 0) with pin 1 at the origin and the
// pins running along +X; the housing, drawn on its own, has its mating
// face (bottom) on z = 0 with contact 1 at the origin, so the header's
// "Mating face" and the housing's meet with the housing seated on the
// header's floor and every contact over its pin.

/**
 * s: { name, pitch, pin: square post, tail: below the board, H: header
 * height, y0, y1: header body from the pin row, side: body beyond the end
 * pins, wall, floor: header floor top, Hh: housing height, contact }.
 */
function jstConnector(s, kind, n) {
  const A = (n - 1) * s.pitch;
  const x0 = -s.side;
  const x1 = A + s.side;
  const W = x1 - x0;
  const D = s.y1 - s.y0;
  const pins = [];
  for (let i = 0; i < n; i++) pins.push(i * s.pitch);
  connector("Pin 1", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
  metric("Pins", n);
  metric("Pitch", s.pitch, "mm");

  if (kind === "header") {
    // Shroud: walls round a cavity down to the floor, two polarising
    // keyways in the front wall (−Y), the lock window in the back wall.
    let shroud = box(W, D, s.H).translate(x0, s.y0, 0)
      .subtract(box(W - 2 * s.wall, D - 2 * s.wall, s.H).translate(x0 + s.wall, s.y0 + s.wall, s.floor));
    for (const x of [0, A]) shroud = shroud.subtract(box(s.key, s.wall + 0.2, s.H - s.floor - 0.6).translate(x - s.key / 2, s.y0 - 0.1, s.floor + 0.6 + 0.01));
    shroud = shroud.subtract(box(Math.min(2.6, A + 1.2), s.wall + 0.2, 1.6).translate(A / 2 - Math.min(2.6, A + 1.2) / 2, s.y1 - s.wall - 0.1, s.H - 3.2));
    // Standoffs under the floor's ends, so flux washes out.
    shroud = shroud.subtract(box(W - 2.4, D + 0.2, 0.3).translate(x0 + 1.2, s.y0 - 0.1, -0.01));
    const posts = union(...pins.map((x) => squarePin(s.pin, -s.tail, s.H - 1.0).translate(x, 0, 0)));
    connector("Board", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
    connector("Mating face", { at: [0, 0, s.floor], axis: [0, 0, 1], x: [1, 0, 0] });
    metric("Height", s.H, "mm");
    metric("Hole diameter", s.hole, "mm");
    return weighed([["Shroud", shroud, "Nylon 66"], ["Pins", posts, "Brass, tin-plated"]]);
  }

  // Housing: fills the header's cavity (0.05 mm clear all round), contact
  // cavities open at the top for the wires, a pin entry under each, the
  // polarising ribs and the lock ramp outside.
  const c = 0.05;
  const hx0 = x0 + s.wall + c;
  const hy0 = s.y0 + s.wall + c;
  const hw = W - 2 * s.wall - 2 * c;
  const hd = D - 2 * s.wall - 2 * c;
  let housing = box(hw, hd, s.Hh).translate(hx0, hy0, 0);
  const cav = s.pitch - 0.7;
  for (const x of pins) {
    housing = housing.subtract(box(cav, cav, s.Hh).translate(x - cav / 2, -cav / 2, 0.8));
    housing = housing.subtract(box(s.pin + 0.4, s.pin + 0.4, 1).translate(x - (s.pin + 0.4) / 2, -(s.pin + 0.4) / 2, -0.1));
  }
  for (const x of [0, A]) housing = housing.add(box(s.key - 0.2, s.wall - 0.1, s.Hh - 1.2).translate(x - (s.key - 0.2) / 2, hy0 - s.wall + 0.15, 1.2));
  const rampW = Math.min(2.4, A + 1.0);
  const yb = hy0 + hd - 0.01;
  const zr = s.H - s.floor;
  const ramp = prismX([[yb, zr - 3.0], [yb + s.wall - 0.14, zr - 1.8], [yb, zr - 1.8]], A / 2 - rampW / 2, A / 2 + rampW / 2);
  housing = housing.add(ramp);
  // Crimp contacts: a box sleeve round each pin, open at both ends.
  const sw = s.pin + 0.5;
  const sleeve = box(sw, sw, s.Hh - 1.6).translate(-sw / 2, -sw / 2, 0.9)
    .subtract(box(sw - 0.4, sw - 0.4, s.Hh + 2).translate(-(sw - 0.4) / 2, -(sw - 0.4) / 2, -1));
  const contacts = union(...pins.map((x) => sleeve.translate(x, 0, 0)));
  connector("Mating face", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
  connector("Wire entry", { at: [A / 2, 0, s.Hh], axis: [0, 0, 1], x: [1, 0, 0] });
  metric("Height", s.Hh, "mm");
  return weighed([["Housing", housing, "Nylon 66"], ["Contacts", contacts, "Phosphor bronze, tin-plated"]]);
}
