// Structural sections: helpers shared by the cut-to-length profile families
// (EN 10365 I/H/U beams, EN 10056-1 angles, EN 10055 tees, EN 10058/10060
// bars, EN 10219-2 / EN 10210-2 hollow sections, EN 755-9 aluminium).
//
// Every section stands on its end: the cross-section in XY, the length along
// +Z from z = 0 (end A) to z = L (end B). Stock is bought by the metre and
// cut: stock() carries the piece length and the price per metre, so the
// piece's price and mass follow the length the design sets.

/** Steel (EN 10025 / 10219 / 10210, 7850 kg/m³): kg per metre for each mm² of section. */
const STEEL_KG_PER_M_PER_MM2 = 7.85e-3;
/** Aluminium 6060 / 6063 (2700 kg/m³). */
const ALU_KG_PER_M_PER_MM2 = 2.7e-3;

/**
 * Typical merchant prices (USD per kg, 2026, cut to length, ex VAT): the
 * per-metre price is the section's mass per metre at its product's rate.
 * Stockholders' list prices for small quantities; mill prices are lower.
 */
const PRICE_PER_KG = {
  beam: 1.65, // IPE, HE, UPN, UPE
  bar: 1.85, // angles, tees, flats, rounds
  coldHollow: 1.95, // EN 10219 SHS / RHS / CHS
  hotHollow: 2.35, // EN 10210 SHS / RHS / CHS
  aluminium: 7.2, // EN 755 6060 / 6063 extrusions, mill finish
};

/**
 * A closed outline from corner points [x, y, r?]: a corner with r > 0 is
 * rounded by a true tangent arc of radius r (convex or concave, any angle),
 * chorded within the program's tolerance. Points run either way round.
 */
function roundedOutline(corners) {
  const out = [];
  const n = corners.length;
  for (let i = 0; i < n; i++) {
    const [x, y, r = 0] = corners[i];
    if (!(r > 0)) {
      out.push([x, y]);
      continue;
    }
    const [ax, ay] = corners[(i + n - 1) % n];
    const [bx, by] = corners[(i + 1) % n];
    const la = Math.hypot(ax - x, ay - y);
    const lb = Math.hypot(bx - x, by - y);
    const u1 = [(ax - x) / la, (ay - y) / la];
    const u2 = [(bx - x) / lb, (by - y) / lb];
    const half = Math.acos(Math.max(-1, Math.min(1, u1[0] * u2[0] + u1[1] * u2[1]))) / 2;
    const d = r / Math.tan(half); // corner to tangent point
    if (d > la + 1e-9 || d > lb + 1e-9) throw new Error(`roundedOutline: radius ${r} does not fit the corner at (${x}, ${y})`);
    const bis = [u1[0] + u2[0], u1[1] + u2[1]];
    const bl = Math.hypot(bis[0], bis[1]);
    const c = [x + (bis[0] / bl) * (r / Math.sin(half)), y + (bis[1] / bl) * (r / Math.sin(half))];
    const t1 = [x + u1[0] * d, y + u1[1] * d];
    const t2 = [x + u2[0] * d, y + u2[1] * d];
    const a1 = Math.atan2(t1[1] - c[1], t1[0] - c[0]);
    let sweep = Math.atan2(t2[1] - c[1], t2[0] - c[0]) - a1;
    if (sweep > Math.PI) sweep -= 2 * Math.PI;
    if (sweep < -Math.PI) sweep += 2 * Math.PI;
    const steps = Math.max(2, Math.ceil((segmentsFor(r) * Math.abs(sweep)) / (2 * Math.PI)));
    for (let k = 0; k <= steps; k++) {
      const a = a1 + (sweep * k) / steps;
      out.push([c[0] + r * Math.cos(a), c[1] + r * Math.sin(a)]);
    }
  }
  return out;
}

/** The points of a w × h rectangle centred on the origin with corner radius r (counter-clockwise). */
function roundedRectPoints(w, h, r) {
  return roundedOutline([[-w / 2, -h / 2, r], [w / 2, -h / 2, r], [w / 2, h / 2, r], [-w / 2, h / 2, r]]);
}

/** Points of a circle of radius r (counter-clockwise), chorded within tolerance. */
function circlePoints(r) {
  const n = segmentsFor(r);
  return Array.from({ length: n }, (_, k) => [r * Math.cos((2 * Math.PI * k) / n), r * Math.sin((2 * Math.PI * k) / n)]);
}

/**
 * Section properties of outlines (mm): `solid` outlines add, `holes` cut.
 * Area A, centroid (cx, cy), second moments about the centroidal axes
 * parallel to X (Ix) and Y (Iy), from the polygon (Green's theorem).
 */
function sectionProps(solid, holes = []) {
  let A = 0, Sx = 0, Sy = 0, Ixx = 0, Iyy = 0;
  const acc = (pts, sign) => {
    const s = sign * Math.sign(windingArea(pts) || 1);
    for (let i = 0; i < pts.length; i++) {
      const [x0, y0] = pts[i];
      const [x1, y1] = pts[(i + 1) % pts.length];
      const c = (x0 * y1 - x1 * y0) * s;
      A += c / 2;
      Sx += ((x0 + x1) * c) / 6;
      Sy += ((y0 + y1) * c) / 6;
      Ixx += ((y0 * y0 + y0 * y1 + y1 * y1) * c) / 12;
      Iyy += ((x0 * x0 + x0 * x1 + x1 * x1) * c) / 12;
    }
  };
  for (const p of solid) acc(p, 1);
  for (const p of holes) acc(p, -1);
  const cx = Sx / A;
  const cy = Sy / A;
  return { A, cx, cy, Ix: Ixx - A * cy * cy, Iy: Iyy - A * cx * cx };
}

/**
 * Metrics every section shows: area (cm²), second moments (cm⁴) and elastic
 * moduli (cm³) about its centroidal axes, named as Eurocode 3 names them for
 * a member standing this way: y-y parallel to X (an I-beam's strong axis,
 * its depth along Y), z-z parallel to Y.
 */
function sectionMetrics(props, { yMin, yMax, xMin, xMax }) {
  const r2 = (v) => Math.round(v * 100) / 100;
  metric("Section area", r2(props.A / 100), "cm²");
  metric("Iy", r2(props.Ix / 1e4), "cm⁴");
  metric("Iz", r2(props.Iy / 1e4), "cm⁴");
  metric("Wel,y", r2(props.Ix / Math.max(yMax - props.cy, props.cy - yMin) / 1e3), "cm³");
  metric("Wel,z", r2(props.Iy / Math.max(xMax - props.cx, props.cx - xMin) / 1e3), "cm³");
}

/**
 * The bought piece: `designation` ("IPE 200"), `grade` ("S235JR"), cut to
 * `L` mm at `kgPerM`; the per-metre price is the mass at the product's rate
 * (or `pricePerM` when given). Part number "IPE 200 × 3000 S235JR".
 */
function cutToLength({ designation, grade, standard, description, kgPerM, rate, pricePerM, L, sources }) {
  const perM = cents(pricePerM ?? kgPerM * PRICE_PER_KG[rate]);
  stock({
    standard,
    partNumber: `${designation} × ${num(L)} ${grade}`,
    description: `${description}, ${grade}, cut to ${num(L)} mm (${num(Math.round(kgPerM * 100) / 100)} kg/m, ${perM.toFixed(2)} USD/m)`,
    unitPrice: cents((perM * L) / 1000),
    currency: "USD",
    pack: 1,
    massG: Math.round(kgPerM * L * 10) / 10,
    cutLength: L,
    pricePerM: perM,
    sources,
  });
  metric("Length", L, "mm");
  metric("Mass per metre", Math.round(kgPerM * 1000) / 1000, "kg/m");
  metric("Price per metre", perM, "USD/m");
}

/** Suppliers for steel sections: searches for the designation. */
function steelSources(query) {
  return [supplierSearch("Metal Supermarkets", query), supplierSearch("McMaster-Carr", query), supplierSearch("Amazon", query)];
}

/** Suppliers for aluminium extrusions. */
function aluminiumSources(query) {
  return [supplierSearch("Misumi", query), supplierSearch("Metal Supermarkets", query), supplierSearch("McMaster-Carr", query), supplierSearch("Amazon", query)];
}

/**
 * End A (z = 0, facing −Z), end B (z = L, facing +Z) and the member's axis
 * through the section's centroid along +Z: what a frame joins beams by.
 */
function memberConnectors(L, cx = 0, cy = 0) {
  connector("End A", { at: [cx, cy, 0], axis: [0, 0, -1], x: [1, 0, 0] });
  connector("End B", { at: [cx, cy, L], axis: [0, 0, 1], x: [1, 0, 0] });
  connector("Axis", { at: [cx, cy, 0], axis: [0, 0, 1], x: [1, 0, 0] });
}

/** A face of the member, mid-length, facing out along `axis`, the length as its x. */
function faceConnector(name, x, y, L, axis) {
  connector(name, { at: [x, y, L / 2], axis: [axis[0], axis[1], 0], x: [0, 0, 1] });
}

/** The extruded member from an outline (and holes), with its section metrics. */
function member(outline, L, holes = []) {
  const props = sectionProps([outline], holes);
  const xs = outline.map((q) => q[0]);
  const ys = outline.map((q) => q[1]);
  sectionMetrics(props, { xMin: Math.min(...xs), xMax: Math.max(...xs), yMin: Math.min(...ys), yMax: Math.max(...ys) });
  const section = holes.length ? polygon(outline).subtract(...holes.map((h) => polygon(h))) : polygon(outline);
  return { solid: section.extrude(L), props };
}

// ---- Families' builders ----------------------------------------------------------

/**
 * A doubly symmetric I or H section (IPE, HE): depth h along Y, flange
 * width b along X, centred on the origin; web tw, flanges tf, root radius r.
 */
function iSection({ h, b, tw, tf, r }, L) {
  const y0 = h / 2 - tf;
  const outline = roundedOutline([
    [-b / 2, -h / 2], [b / 2, -h / 2], [b / 2, -y0], [tw / 2, -y0, r], [tw / 2, y0, r], [b / 2, y0], [b / 2, h / 2],
    [-b / 2, h / 2], [-b / 2, y0], [-tw / 2, y0, r], [-tw / 2, -y0, r], [-b / 2, -y0],
  ]);
  const { solid, props } = member(outline, L);
  memberConnectors(L);
  faceConnector("Top flange", 0, h / 2, L, [0, 1]);
  faceConnector("Bottom flange", 0, -h / 2, L, [0, -1]);
  faceConnector("Top flange underside +X", (tw / 2 + b / 2) / 2, y0, L, [0, -1]);
  faceConnector("Top flange underside -X", -(tw / 2 + b / 2) / 2, y0, L, [0, -1]);
  faceConnector("Bottom flange top +X", (tw / 2 + b / 2) / 2, -y0, L, [0, 1]);
  faceConnector("Bottom flange top -X", -(tw / 2 + b / 2) / 2, -y0, L, [0, 1]);
  faceConnector("Web +X", tw / 2, 0, L, [1, 0]);
  faceConnector("Web -X", -tw / 2, 0, L, [-1, 0]);
  return { solid, props };
}

/**
 * A channel (UPN, UPE, aluminium U): back of the web on x = 0, flanges
 * toward +X, depth h along Y centred on y = 0. The flanges' inner faces
 * slope `slope` (UPN: 8 %, 5 % over 300) about their thickness tf, measured
 * at x = `tAt` from the back of the web (default halfway along the
 * flange's inside); root radius r1, toe radius r2.
 */
function channelSection({ h, b, tw, tf, r1, r2 = 0, slope = 0, tAt = (b + tw) / 2 }, L) {
  const t = (x) => tf + slope * (tAt - x); // flange thickness at x
  const outline = roundedOutline([
    [0, -h / 2], [b, -h / 2], [b, -h / 2 + t(b), r2], [tw, -h / 2 + t(tw), r1],
    [tw, h / 2 - t(tw), r1], [b, h / 2 - t(b), r2], [b, h / 2], [0, h / 2],
  ]);
  const { solid, props } = member(outline, L);
  memberConnectors(L, props.cx, 0);
  faceConnector("Web back", 0, 0, L, [-1, 0]);
  faceConnector("Web inside", tw, 0, L, [1, 0]);
  faceConnector("Top flange", b / 2, h / 2, L, [0, 1]);
  faceConnector("Bottom flange", b / 2, -h / 2, L, [0, -1]);
  metric("Centroid from web back", Math.round(props.cx * 100) / 100, "mm");
  return { solid, props };
}

/**
 * An angle: the heel on the origin, leg a up +Y, leg b along +X, both t
 * thick; root radius r1, toe radius r2 (inside corner of each leg's tip).
 */
function angleSection({ a, b, t, r1, r2 = 0 }, L) {
  const outline = roundedOutline([[0, 0], [b, 0], [b, t, r2], [t, t, r1], [t, a, r2], [0, a]]);
  const { solid, props } = member(outline, L);
  memberConnectors(L, props.cx, props.cy);
  faceConnector("Leg A back", 0, a / 2, L, [-1, 0]);
  faceConnector("Leg A inside", t, (a + t) / 2, L, [1, 0]);
  faceConnector("Leg B back", b / 2, 0, L, [0, -1]);
  faceConnector("Leg B inside", (b + t) / 2, t, L, [0, 1]);
  metric("Centroid ex", Math.round(props.cx * 100) / 100, "mm");
  metric("Centroid ey", Math.round(props.cy * 100) / 100, "mm");
  return { solid, props };
}

/**
 * A hollow section: SHS / RHS (b along X, h along Y, wall t, outer corner
 * radius ro, inner ri) or CHS (outside diameter d), centred on the origin.
 * The rounded rectangles are true arcs (offsets of rectangles), exact in STEP.
 */
function hollowSection({ b, h, d, t, ro = 0, ri = 0 }, L) {
  let section, outer, inner;
  if (d) {
    section = circle(d / 2).subtract(circle(d / 2 - t));
    outer = circlePoints(d / 2);
    inner = circlePoints(d / 2 - t);
  } else {
    const rr = (w, hh, r) => (r > 0 ? rect(w - 2 * r, hh - 2 * r, { center: true }).offset(r, { join: "round" }) : rect(w, hh, { center: true }));
    section = rr(b, h, ro).subtract(rr(b - 2 * t, h - 2 * t, ri));
    outer = roundedRectPoints(b, h, ro);
    inner = roundedRectPoints(b - 2 * t, h - 2 * t, ri);
  }
  const props = sectionProps([outer], [inner]);
  const w = d || b;
  const hh = d || h;
  sectionMetrics(props, { xMin: -w / 2, xMax: w / 2, yMin: -hh / 2, yMax: hh / 2 });
  memberConnectors(L);
  if (!d) {
    faceConnector("Face +X", b / 2, 0, L, [1, 0]);
    faceConnector("Face -X", -b / 2, 0, L, [-1, 0]);
    faceConnector("Face +Y", 0, h / 2, L, [0, 1]);
    faceConnector("Face -Y", 0, -h / 2, L, [0, -1]);
  } else {
    faceConnector("Surface +X", d / 2, 0, L, [1, 0]);
    faceConnector("Surface +Y", 0, d / 2, L, [0, 1]);
  }
  return { solid: section.extrude(L), props };
}
