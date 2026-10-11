// Involute spur gears, computed from their definition.
//
// A tooth flank is the involute of the base circle (radius r·cos α): the path
// of a taut string unwound from it. With roll angle t a flank point is
//   r_b · (cos t + t·sin t,  sin t − t·cos t)
// and its radius of curvature is r_b·t, so the chord step that keeps the
// polygon within `tolerance` of the true curve is dt ≤ √(8·tol / (r_b·t)).
// Teeth are thinned by half the backlash each, so mating gears never touch.

/** Polar angle of the involute at radius ρ (from where it leaves the base circle). */
const inv = (rb, rho) => {
  const t = Math.sqrt(Math.max(0, (rho / rb) ** 2 - 1));
  return t - Math.atan(t);
};

/**
 * Outline of an external spur gear, centred on the origin, tooth 0 on +X.
 * m: module (mm), z: teeth, alpha: pressure angle (deg), backlash: mm along the pitch circle.
 */
function gearOutline({ m, z, alpha = 20, backlash = 0.1 }) {
  const a = rad(alpha);
  const r = (m * z) / 2;
  const rb = r * Math.cos(a);
  const ra = r + m;
  const rf = r - 1.25 * m;
  // Half the tooth's angular thickness at the pitch circle.
  const half = Math.PI / (2 * z) - backlash / (4 * r);
  // Angle of the flank at radius ρ, measured from the tooth centre.
  const flank = (rho) => half + inv(rb, r) - inv(rb, rho);

  // Flank radii, stepped by roll angle so every chord stays within tolerance.
  const rStart = Math.max(rb, rf);
  const tStart = Math.sqrt(Math.max(0, (rStart / rb) ** 2 - 1));
  const tEnd = Math.sqrt((ra / rb) ** 2 - 1);
  const radii = [];
  for (let t = tStart; ; ) {
    radii.push(rb * Math.sqrt(1 + t * t));
    if (t >= tEnd) break;
    const dt = Math.sqrt((8 * tolerance) / (rb * Math.max(t, 1e-3)));
    t = Math.min(tEnd, t + Math.max(dt, 1e-4));
  }

  const arc = (radius, from, to) => {
    const n = Math.max(1, Math.ceil(((to - from) / (2 * Math.PI)) * segmentsFor(radius)));
    const pts = [];
    for (let i = 1; i < n; i++) {
      const th = from + ((to - from) * i) / n;
      pts.push([radius * Math.cos(th), radius * Math.sin(th)]);
    }
    return pts;
  };
  const at = (radius, th) => [radius * Math.cos(th), radius * Math.sin(th)];

  const pts = [];
  const pitch = (2 * Math.PI) / z;
  for (let k = 0; k < z; k++) {
    const c = k * pitch;
    // Below the base circle the flank runs radially down to the root.
    if (rf < rb) pts.push(at(rf, c - flank(rb)));
    for (const rho of radii) pts.push(at(rho, c - flank(rho)));
    pts.push(...arc(ra, c - flank(ra), c + flank(ra)));
    for (const rho of [...radii].reverse()) pts.push(at(rho, c + flank(rho)));
    if (rf < rb) pts.push(at(rf, c + flank(rb)));
    // Root circle to the next tooth.
    const from = c + flank(rStart);
    pts.push(...arc(rf, from, c + pitch - flank(rStart)));
  }
  return polygon(pts);
}

/** Pitch radius (mm). */
const pitchRadius = (m, z) => (m * z) / 2;

/**
 * A gear blank: toothed rim, bore with a keyway (DIN 6885 proportions for
 * small shafts) and an optional hub on one side.
 */
function gear({ m, z, width, bore, hub = 0, hubLength = 0, backlash = 0.1 }) {
  let g = gearOutline({ m, z, backlash }).extrude(width);
  if (hub > 0 && hubLength > 0) g = g.add(cylinder(hubLength + 0.01, hub / 2).translate(0, 0, width - 0.01));
  const key = Math.max(2, Math.round(bore / 4));
  const length = width + hubLength + 2;
  const hole = cylinder(length, bore / 2).translate(0, 0, -1).add(box(key, bore / 2 + key / 2, length).translate(-key / 2, 0, -1));
  return g.subtract(hole);
}
