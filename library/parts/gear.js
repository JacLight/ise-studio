// Involute gears: one gear, used again with other settings (teeth, module,
// width, helix), as a designer clones a gear and changes its tooth count.
//
//   const { gear, ring, meshTurn, ringTurn, centerDistance } = use("gear");
//   const sun = gear({ module: 2, teeth: 12, width: 10, helix: 25, herringbone: true });
//   const planet = gear({ module: 2, teeth: 18, width: 10, helix: 25, herringbone: true, hand: -1 });
//   const annulus = ring({ module: 2, teeth: 48, width: 10, rim: 6, helix: 25, herringbone: true, hand: -1 });
//
// A tooth flank is the involute of the base circle (radius r·cos α): with roll
// angle t a flank point is r_b·(cos t + t·sin t, sin t − t·cos t), stepped so
// every chord stays within `tolerance` of the curve. Teeth are thinned by half
// the backlash each (mm along the pitch circle), so mating gears never touch.
// Every gear has tooth 0 on +X, centred on the origin, from z = 0 up.

const inv = (rb, rho) => {
  const t = Math.sqrt(Math.max(0, (rho / rb) ** 2 - 1));
  return t - Math.atan(t);
};

/** The outline of a toothed wheel: tip `ra`, root `rf`, half a tooth's angle at the pitch circle `half`. */
function toothed({ m, z, alpha, ra, rf, half }) {
  const a = rad(alpha);
  const r = (m * z) / 2;
  const rb = r * Math.cos(a);
  const flank = (rho) => half + inv(rb, r) - inv(rb, rho);
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
  const at = (radius, th) => [radius * Math.cos(th), radius * Math.sin(th)];
  const arc = (radius, from, to) => {
    const n = Math.max(1, Math.ceil(((to - from) / (2 * Math.PI)) * segmentsFor(radius)));
    const pts = [];
    for (let i = 1; i < n; i++) pts.push(at(radius, from + ((to - from) * i) / n));
    return pts;
  };
  const pts = [];
  const pitch = (2 * Math.PI) / z;
  for (let k = 0; k < z; k++) {
    const c = k * pitch;
    if (rf < rb) pts.push(at(rf, c - flank(rb)));
    for (const rho of radii) pts.push(at(rho, c - flank(rho)));
    pts.push(...arc(ra, c - flank(ra), c + flank(ra)));
    for (const rho of [...radii].reverse()) pts.push(at(rho, c + flank(rho)));
    if (rf < rb) pts.push(at(rf, c + flank(rb)));
    pts.push(...arc(rf, c + flank(rStart), c + pitch - flank(rStart)));
  }
  return polygon(pts);
}

const pitchRadius = (module, teeth) => (module * teeth) / 2;

/** Settings checked before any geometry: a wrong one is named with what is accepted. */
function settings(what, given, accepted) {
  if (given === null || typeof given !== "object") throw new Error(`${what}: give its settings as an object, e.g. ${what}({ module: 2, teeth: 18, width: 10 })`);
  const unknown = Object.keys(given).filter((k) => !accepted.includes(k));
  if (unknown.length) throw new Error(`${what}: no setting ${unknown.map((k) => `“${k}”`).join(", ")} (it takes ${accepted.join(", ")})`);
  const positive = (k) => {
    if (!(typeof given[k] === "number" && given[k] > 0)) throw new Error(`${what}: ${k} must be a number above 0 (mm${k === "teeth" ? ", or a whole count" : ""}), not ${JSON.stringify(given[k])}`);
  };
  positive("module");
  positive("teeth");
  if (!Number.isInteger(given.teeth) || given.teeth < 5) throw new Error(`${what}: teeth must be a whole number of 5 or more, not ${given.teeth}`);
  if ("width" in given || what !== "outline") positive("width");
  if ("hand" in given && given.hand !== 1 && given.hand !== -1) throw new Error(`${what}: hand is 1 or -1 (the twist's direction; mating external gears take opposite hands, a ring its planets' hand), not ${JSON.stringify(given.hand)}`);
  for (const k of ["helix", "backlash", "bore", "rim", "outside", "pressureAngle"]) {
    if (k in given && !(typeof given[k] === "number" && given[k] >= 0)) throw new Error(`${what}: ${k} must be a number, 0 or more, not ${JSON.stringify(given[k])}`);
  }
  if ((given.helix ?? 0) >= 60) throw new Error(`${what}: helix is the tooth angle at the pitch circle in degrees, below 60 (25–30 is usual), not ${given.helix}`);
}

/** An external gear's outline (2D), tooth 0 on +X. */
function outline(options) {
  settings("outline", options, ["module", "teeth", "pressureAngle", "backlash"]);
  const { module, teeth, pressureAngle = 20, backlash = 0.1 } = options;
  const r = pitchRadius(module, teeth);
  return toothed({ m: module, z: teeth, alpha: pressureAngle, ra: r + module, rf: r - 1.25 * module, half: Math.PI / (2 * teeth) - backlash / (4 * r) });
}

/** The space inside a ring gear (2D): shaped as an external gear whose teeth are the ring's gaps. */
function ringSpace({ module, teeth, pressureAngle = 20, backlash = 0.1 }) {
  const r = pitchRadius(module, teeth);
  return toothed({ m: module, z: teeth, alpha: pressureAngle, ra: r + 1.25 * module, rf: r - module, half: Math.PI / (2 * teeth) + backlash / (4 * r) });
}

/** A 2D outline made solid over `width`: straight, helical (helix ° at the pitch circle, `hand` ±1) or herringbone. */
function teethSolid(shape, r, width, helix, hand, herringbone) {
  if (!helix) return shape.extrude(width);
  const run = herringbone ? width / 2 : width;
  const twist = ((run * Math.tan(rad(helix))) / r) * (180 / Math.PI) * hand;
  const slices = Math.max(2, Math.ceil(Math.abs(twist) / 1.5));
  const first = shape.extrude(run, { twist, slices });
  if (!herringbone) return first;
  return first.add(shape.rotate(twist).extrude(run, { twist: -twist, slices }).translate(0, 0, run));
}

/**
 * An external gear. module (mm), teeth, width (mm); pressureAngle (20°);
 * backlash (mm, 0.1); helix (° at the pitch circle, 0: spur), hand (1 or -1:
 * mating external gears take opposite hands), herringbone; bore (diameter, mm).
 */
function gear(options) {
  settings("gear", options, ["module", "teeth", "width", "pressureAngle", "backlash", "helix", "hand", "herringbone", "bore"]);
  const { module, teeth, width, pressureAngle = 20, backlash = 0.1, helix = 0, hand = 1, herringbone = false, bore = 0 } = options;
  const r = pitchRadius(module, teeth);
  if (bore >= 2 * (r - 1.25 * module)) throw new Error(`gear: a ${bore} mm bore cuts through the roots (root diameter ${(2 * (r - 1.25 * module)).toFixed(2)} mm)`);
  let g = teethSolid(outline({ module, teeth, pressureAngle, backlash }), r, width, helix, hand, herringbone);
  if (bore > 0) g = g.subtract(cylinder(width + 2, bore / 2).translate(0, 0, -1));
  return g;
}

/**
 * A ring (internal) gear: teeth inside a rim `rim` mm thick outside their
 * roots (or outer diameter `outside`). It takes its planets' hand.
 */
function ring(options) {
  settings("ring", options, ["module", "teeth", "width", "rim", "outside", "pressureAngle", "backlash", "helix", "hand", "herringbone"]);
  const { module, teeth, width, rim = 5, outside = 0, pressureAngle = 20, backlash = 0.1, helix = 0, hand = 1, herringbone = false } = options;
  const r = pitchRadius(module, teeth);
  const roots = 2 * (r + 1.25 * module);
  if (outside > 0 && outside <= roots) throw new Error(`ring: outside is the ring's outer diameter, more than its tooth roots' ${roots.toFixed(2)} mm, not ${outside} (or give rim, the wall outside the roots)`);
  const outer = outside > 0 ? outside / 2 : roots / 2 + rim;
  const space = teethSolid(ringSpace({ module, teeth, pressureAngle, backlash }), r, width + 0.02, helix, hand, herringbone).translate(0, 0, -0.01);
  return cylinder(width, outer).subtract(space);
}

/** Distance between the centres of two meshing gears (a ring and its planet: `internal`). */
const centerDistance = (module, teeth1, teeth2, internal = false) => (module * (internal ? teeth2 - teeth1 : teeth1 + teeth2)) / 2;

/**
 * How far (°) to turn gear B, centred at angle `at` (°) round gear A, for its
 * teeth to fit A's when A is turned `turnA` (°). Both external.
 */
const meshTurn = (teethA, teethB, at, turnA = 0) => at + 180 - 180 / teethB + ((at - turnA) * teethA) / teethB;

/**
 * How far (°) to turn a ring gear (teeth `teethRing`, centred on the origin)
 * for a planet (teeth `teethPlanet`) centred at angle `at` (°), turned
 * `turnPlanet` (°), to fit it. Planets spaced evenly fit one ring when
 * (sun teeth + ring teeth) / planets is whole.
 */
const ringTurn = (teethPlanet, teethRing, at, turnPlanet) => at - ((at - turnPlanet) * teethPlanet) / teethRing;

return { gear, ring, outline, ringSpace, pitchRadius, centerDistance, meshTurn, ringTurn };
