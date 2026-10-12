// GT2 (Gates PowerGrip 2GT, 2 mm pitch) helpers shared by gt2_pulley and
// gt2_idler.
//
// Source: Gates PowerGrip GT2 design manual — pitch 2 mm, pitch line
// differential 0.254 mm (so the toothed outside diameter is the pitch
// diameter less 0.508), belt tooth height 0.75 mm. The pulley groove is
// modelled as in the widely used open parametric pulley (droftarts), after
// the Gates profile: a round-bottomed groove 0.764 mm deep with a 0.555 mm
// root radius, opening to about 1.38 mm at the outside diameter.

const GT2_PITCH = 2;
const GT2_PLD = 0.254;

/** Pitch diameter of an N-tooth GT2 pulley. */
const gt2Pitch = (teeth) => (teeth * GT2_PITCH) / Math.PI;
/** Toothed outside diameter. */
const gt2Outside = (teeth) => gt2Pitch(teeth) - 2 * GT2_PLD;

/** The toothed outline of an N-tooth GT2 pulley (2D, centred on the origin). */
function gt2Profile(teeth) {
  const R = gt2Outside(teeth) / 2;
  const depth = 0.764;
  const root = 0.555;
  const half = 0.69; // half the opening at the outside diameter
  const rc = R - depth + root;
  const slope = (half - root) / (R - rc);
  const reach = R + 0.3;
  const groove = circle(root)
    .translate(rc, 0)
    .add(polygon([[rc, -root], [reach, -(root + slope * (reach - rc))], [reach, root + slope * (reach - rc)], [rc, root]]));
  const grooves = [];
  for (let i = 0; i < teeth; i++) grooves.push(groove.rotate((360 * i) / teeth));
  return grooves.reduce((acc, g) => acc.subtract(g), circle(R));
}

/**
 * A shielded miniature ball bearing (…ZZ) as one solid: inner and outer
 * rings with raceway grooves, its balls and the two steel shields, axis
 * along +Z from z = 0 to z = B.
 */
function miniBearing(d, D, B, Z, Dw) {
  const rp = (d + D) / 4;
  const rg = 0.52 * Dw;
  const ri = rp - 0.33 * Dw;
  const ro = rp + 0.33 * Dw;
  const zg = Math.sqrt(rg * rg - (0.33 * Dw) ** 2);
  const arc = (side) => {
    const a = Math.asin(zg / rg);
    const pts = [];
    for (let i = 0; i <= 8; i++) {
      const t = -a + (2 * a * i) / 8;
      pts.push([rp + side * rg * Math.cos(t), B / 2 + rg * Math.sin(t)]);
    }
    return pts;
  };
  const inner = polygon([[d / 2, 0], [ri, 0], ...arc(-1), [ri, B], [d / 2, B]]).revolve(360);
  const outer = polygon([[ro, 0], [D / 2, 0], [D / 2, B], [ro, B], ...arc(1).reverse()]).revolve(360);
  const shield = polygon([[ri + 0.05, 0], [ro, 0], [ro, 0.2], [ri + 0.05, 0.2]]).revolve(360);
  const list = [inner, outer, shield.translate(0, 0, 0.1), shield.translate(0, 0, B - 0.3)];
  for (let i = 0; i < Z; i++) {
    const a = (2 * Math.PI * i) / Z;
    list.push(sphere(Dw / 2 - 0.005).translate(rp * Math.cos(a), rp * Math.sin(a), B / 2));
  }
  return union(list);
}

/** An ISO 4029 cup-point socket set screw M×l along +Z from its tip at z = 0. */
function setScrew(M, l) {
  const s = { 3: 1.5, 4: 2, 5: 2.5, 6: 3 }[M];
  return externalThread({ d: M, pitch: coarsePitch(M), z0: 0, z1: l, tip: "bottom" }).subtract(hexSocket(s, 0.6 * l, l));
}

// ---- Belts ------------------------------------------------------------------------

// GT2 belt section (Gates PowerGrip 2MGT): backing 0.63 mm, teeth 0.75 mm
// deep (1.38 overall), the pitch line (in the glass-fibre cords) 0.254 mm
// outside the land the teeth stand on. The tooth is drawn as the usual
// rounded 2GT shape: 1.2 mm wide at the land, a 0.5 mm tip radius.
const GT2_BACK = 0.63;
const GT2_TOOTH = 0.75;

/** One belt tooth as points (u along the belt, w towards the pulley), from u = +0.6 round to −0.6. */
function gt2ToothPoints() {
  const r = 0.5;
  const wc = GT2_TOOTH - r;
  const n = Math.max(4, Math.ceil(segmentsFor(r) / 2));
  const pts = [[0.6, 0], [0.5, wc]];
  for (let i = 1; i < n; i++) {
    const t = (Math.PI * i) / n;
    pts.push([r * Math.cos(t), wc + r * Math.sin(t)]);
  }
  pts.push([-0.5, wc], [-0.6, 0]);
  return pts;
}
