// Rolling bearing helpers shared by bearing_deep_groove, bearing_flanged and
// bearing_thrust.
//
// Radial ball bearings: raceways are true circular grooves (radius 0.52·Dw,
// the usual osculation), lands at dm ∓ 0.66·Dw, seals or shields sit 0.15 mm
// in from each face in a recess of the outer ring, and a pressed-steel
// ribbon cage holds the balls apart. Axis +Z, face A on z = 0, face B on
// z = B; a flange (flanged bearings) is on the outer ring at face A.

/**
 * The parts of a single-row radial ball bearing: { inner, outer, balls,
 * cage, closures }. `closure` "2RS" (rubber seals) or "ZZ" (steel shields);
 * `flange` [Df, Bf] puts a flange of that diameter and width on face A.
 */
function radialBallBearing({ d, D, B, Z, Dw, closure = "2RS", flange = null }) {
  const dm = (d + D) / 2; // pitch diameter
  const rp = dm / 2;
  const rg = 0.52 * Dw; // groove radius (osculation 52 %)
  const ri = rp - 0.33 * Dw; // inner ring land
  const ro = rp + 0.33 * Dw; // outer ring land
  const zg = Math.sqrt(rg * rg - (0.33 * Dw) ** 2); // where the groove meets the land
  const c = Math.min(0.3, 0.06 * (D - d)); // corner chamfers
  const ts = Math.min(1, Math.max(0.3, 0.09 * B)); // seal thickness
  const tz = Math.min(0.2, ts); // shield thickness (pressed steel)
  const rs = ro + 0.4 * (D / 2 - ro); // seal recess in the outer ring
  const zs = 0.15 + ts; // seal recess depth from each face

  /** Groove arc points about the ball centre, on the inner (-1) or outer (+1) side, from z low to high. */
  const groove = (side) => {
    const a = Math.asin(zg / rg);
    const n = Math.max(6, Math.ceil(segmentsFor(rg) * (2 * a) / (2 * Math.PI)));
    const pts = [];
    for (let i = 0; i <= n; i++) {
      const t = -a + (2 * a * i) / n;
      pts.push([rp + side * rg * Math.cos(t), B / 2 + rg * Math.sin(t)]);
    }
    return pts;
  };

  const inner = polygon([
    [d / 2 + c, 0],
    [ri, 0],
    ...groove(-1),
    [ri, B],
    [d / 2 + c, B],
    [d / 2, B - c],
    [d / 2, c],
  ]).revolve(360);

  // Face A of the outer ring: a plain chamfered corner, or the flange.
  const faceA = flange
    ? (() => {
        const [Df, Bf] = flange;
        const cf = Math.min(0.2, 0.15 * Bf);
        return [[Df / 2 - cf, 0], [Df / 2, cf], [Df / 2, Bf - cf], [Df / 2 - cf, Bf], [D / 2, Bf]];
      })()
    : [[D / 2 - c, 0], [D / 2, c]];
  const outer = polygon([
    [rs, 0],
    ...faceA,
    [D / 2, B - c],
    [D / 2 - c, B],
    [rs, B],
    [rs, B - zs],
    [ro, B - zs],
    ...groove(1).reverse(),
    [ro, zs],
    [rs, zs],
  ]).revolve(360);

  const balls = [];
  for (let i = 0; i < Z; i++) {
    const a = (2 * Math.PI * i) / Z;
    balls.push(sphere(Dw / 2).translate(rp * Math.cos(a), rp * Math.sin(a), B / 2));
  }

  // Seals: rubber-faced discs from just over the inner land out into the
  // recess. Shields: thinner pressed-steel discs in the same recess, against
  // its outer shoulder.
  const t = closure === "ZZ" ? tz : ts;
  const disc = polygon([[ri + 0.08, 0], [rs, 0], [rs, t], [ri + 0.08, t]]).revolve(360);
  const closures = disc.translate(0, 0, zs - t).add(disc.translate(0, 0, B - zs));

  // Cage: a pressed-steel ribbon band round the pitch circle with a pocket per ball.
  const band = polygon([[rp - 0.18 * Dw, B / 2 - 0.35 * Dw], [rp + 0.18 * Dw, B / 2 - 0.35 * Dw], [rp + 0.18 * Dw, B / 2 + 0.35 * Dw], [rp - 0.18 * Dw, B / 2 + 0.35 * Dw]]).revolve(360);
  const pockets = [];
  for (let i = 0; i < Z; i++) {
    const a = (2 * Math.PI * i) / Z;
    pockets.push(sphere(Dw / 2 + 0.03).translate(rp * Math.cos(a), rp * Math.sin(a), B / 2));
  }
  const cage = band.subtract(union(pockets));

  return { inner, outer, balls: union(balls), cage, closures };
}

/** The usual connectors of a radial bearing: bore axis, outer seat, the two faces. */
function bearingConnectors(D, B) {
  connector("Bore", { at: [0, 0, B / 2], axis: [0, 0, 1] });
  connector("Outer seat", { at: [D / 2, 0, B / 2], axis: [1, 0, 0], x: [0, 0, 1] });
  connector("Face A", { at: [0, 0, 0], axis: [0, 0, -1] });
  connector("Face B", { at: [0, 0, B], axis: [0, 0, 1] });
}
