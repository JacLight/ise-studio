// Helpers shared by the electronics families (connectors, switches, LEDs).
//
// Conventions: a board-mounted part stands on the board's top face at
// z = 0 with its leads running down −Z through a 1.6 mm board; a
// panel-mounted part has the panel face it seats on at z = 0. The "Board"
// connector is that seat (axis −Z, into the board), so a fixed joint puts it
// on a board's top face; "Pin 1" marks the footprint's first pin.
//
// Masses: makers publish none for parts this small; each is worked out from
// the modelled geometry at the densities below (the same as the families'
// manifests list).

const ELEC_DENSITY = {
  "Nylon 66": 1.14,
  "Nylon 66 (black)": 1.14,
  "PBT (black)": 1.31,
  "PBT (white)": 1.31,
  "LCP (black)": 1.4,
  "Brass, tin-plated": 8.5,
  "Phosphor bronze, gold-plated": 8.8,
  "Phosphor bronze, tin-plated": 8.8,
  "Steel, nickel-plated": 7.85,
  "Stainless steel": 7.93,
  "Copper alloy, silver-plated": 8.9,
  "Epoxy lens (red)": 1.15,
  "Epoxy lens (green)": 1.15,
  "Epoxy lens (yellow)": 1.15,
  "Epoxy lens (blue)": 1.15,
  "Epoxy lens (white)": 1.15,
  "Polyamide (black)": 1.14,
  "Polycarbonate (red)": 1.2,
  "ABS (black)": 1.05,
};

/**
 * The parts, each [name, solid, material], and their total mass (g) from
 * the volumes; parts with a null solid are left out.
 */
function weighed(list) {
  const parts = [];
  let grams = 0;
  for (const [name, solid, material] of list) {
    if (!solid) continue;
    const rho = ELEC_DENSITY[material];
    if (rho === undefined) throw new Error(`no density for ${material}`);
    grams += (solid.volume * rho) / 1000;
    parts.push(part(name, solid, { material }));
  }
  return { parts, massG: Math.round(grams * 1000) / 1000 };
}

/**
 * A square pin `s` across from z0 to z1 with its ends pointed the way pins
 * are (a 0.25 mm taper to 0.45 s): a prism and two frustums, exact in STEP.
 */
function squarePin(s, z0, z1, { tipBottom = true, tipTop = true } = {}) {
  const c = Math.min(0.25, (z1 - z0) / 4);
  const a = z0 + (tipBottom ? c : 0);
  const b = z1 - (tipTop ? c : 0);
  let pin = rect(s, s, { center: true }).extrude(b - a).translate(0, 0, a);
  if (tipBottom) pin = pin.add(rect(0.45 * s, 0.45 * s, { center: true }).extrude(c + 0.001, { scaleTop: 1 / 0.45 }).translate(0, 0, z0));
  if (tipTop) pin = pin.add(rect(s, s, { center: true }).extrude(c + 0.001, { scaleTop: 0.45 }).translate(0, 0, b - 0.001));
  return pin;
}

/** A prism of the outline `yz` ([y, z] points) run along X from x0 to x1. */
function prismX(yz, x0, x1) {
  return polygon(yz.map(([y, z]) => [-z, y])).extrude(x1 - x0).rotate(0, 90, 0).translate(x0, 0, 0);
}

/** A prism of the outline `xz` ([x, z] points) run along Y from y0 to y1. */
function prismY(xz, y0, y1) {
  return polygon(xz.map(([x, z]) => [x, -z])).extrude(y1 - y0).rotate(-90, 0, 0).translate(0, y0, 0);
}

/** A flat rectangle with semicircular ends (a stadium), `w` × `h`, centred. */
function stadium(w, h) {
  const r = h / 2;
  return rect(w - h, h, { center: true }).add(circle(r).translate(-(w - h) / 2, 0)).add(circle(r).translate((w - h) / 2, 0));
}

/** A rounded rectangle `w` × `h` with corner radius `r`, centred. */
function roundedRect(w, h, r) {
  return rect(w - 2 * r, h, { center: true }).add(rect(w, h - 2 * r, { center: true }))
    .add(...[[-1, -1], [1, -1], [1, 1], [-1, 1]].map(([sx, sy]) => circle(r).translate(sx * (w / 2 - r), sy * (h / 2 - r))));
}
