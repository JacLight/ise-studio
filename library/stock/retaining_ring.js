// Retaining ring (circlip) with lugs, spring steel C75S black phosphated:
// the shape DIN 471 (for shafts, lugs outside) and DIN 472 (for bores, lugs
// inside) share. The standard's own file runs first and sets STANDARD,
// SHAFT (true for DIN 471) and RING (its table).
//
// Free state as bought: the ring's free diameter d3 (inner for a shaft
// ring, outer for a bore ring) round the axis, its radial width b where it
// is widest (opposite the gap) tapering to about 0.6 b at the lugs; the
// lugs a wide with the pliers' holes d5 in them; the gap at +Y.
//
// Lies on its bottom face at z = 0, top face at z = s, axis along +Z.
// Mass: this nominal geometry at 7.85 g/cm³.

const nominal = p.d;
const [s, d3, a, b, d5, d2, m] = lookup(RING, nominal, STANDARD);
const e = 0.2 * b; // the band's eccentricity: b at the back, 0.6 b at the lugs
const gap = Math.max(0.8, 0.12 * nominal);
const re = a / 2; // the lug's radius
// The band and where the lugs sit (their centres rc from the axis).
const free = d3 / 2;
const band = SHAFT
  ? circle(free + 0.8 * b).translate(0, -e).subtract(circle(free))
  : circle(free).subtract(circle(free - 0.8 * b).translate(0, e));
const rc = SHAFT ? free + re : free - re;
const phi = Math.asin((re + gap / 2) / rc);
const along = rad(25); // the lug blends into the band this far round
const rb = 0.3 * b; // the blend's radius, inside the band's narrow end
const rbc = SHAFT ? free + rb : free - rb;
const lug = (sign) => {
  const at = (r, ang) => [sign * r * Math.sin(ang), r * Math.cos(ang)];
  return circle(re).translate(...at(rc, phi)).add(circle(rb).translate(...at(rbc, phi + along))).hull();
};
let outline = band.add(lug(1), lug(-1));
outline = SHAFT ? outline.subtract(circle(free)) : outline.intersect(circle(free));
outline = outline
  .subtract(rect(gap, free + a + 2).translate(-gap / 2, 0))
  .subtract(circle(d5 / 2).translate(rc * Math.sin(phi), rc * Math.cos(phi)))
  .subtract(circle(d5 / 2).translate(-rc * Math.sin(phi), rc * Math.cos(phi)));
const ring = outline.extrude(s);

const massG = 7.85e-3 * ring.volume;
const what = SHAFT ? "shaft" : "bore";
stock({
  standard: STANDARD,
  partNumber: `${STANDARD} ${num(nominal)}×${num(s)}`,
  description: `Retaining ring (circlip) for a Ø${num(nominal)} ${what}, ${num(s)} mm thick, spring steel: groove Ø${num(d2)} × ${num(m)} wide`,
  unitPrice: lookup(RING_PRICE, nominal, "price"),
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 10000) / 10000,
  sources: [
    supplierSearch("McMaster-Carr", SHAFT ? "external retaining rings" : "internal retaining rings"),
    supplierSearch("Misumi", `retaining ring ${STANDARD} ${num(nominal)}`),
    supplierSearch("AliExpress", `${STANDARD} ${num(nominal)}mm circlip ${SHAFT ? "external" : "internal"}`),
  ],
});
metric(SHAFT ? "Shaft Ø" : "Bore Ø", nominal, "mm");
metric("Groove Ø", d2, "mm");
metric("Groove width", m, "mm");
metric("Thickness", s, "mm");
metric("Pliers hole Ø", d5, "mm");

connector("Bottom face", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top face", { at: [0, 0, s], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, 0], axis: [0, 0, 1] });
return ring;
