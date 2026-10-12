// Helical compression spring, round music wire, ends closed and ground
// (the sizes sold loose and in assortment boxes: wire × OD × free length).
//
// Sources: EN 13906-1 (cylindrical helical compression springs: rate
// R = G d⁴ / (8 D³ n), D the mean coil diameter, n the active coils; closed
// and ground ends add one dead coil each, n_t = n + 2; solid length
// L_c ≤ n_t · d), EN 15800 tolerances; sizes as stocked in metric
// assortments and by Lee Spring / McMaster-Carr (e.g. 0.8 × 6 × 20 mm,
// 1.0 × 8 × 20 mm, 1.5 × 12 × 40 mm).
//
// Geometry: the wire's centre line is a helix of mean diameter OD − d. Each
// end has one dead coil at a pitch of 1.05 d (closed: the coils almost touch;
// a hair open so the solid stays manifold), the active coils between at the
// pitch that gives the free length, and both ends are ground flat across a
// half turn. Stands on its ground end on z = 0, axis along +Z, free length
// to z = L. Mass: worked out from this geometry at 7.85 g/cm³.

const d = p.d;
const OD = p.od;
const L = p.length;
const nt = p.coils;
const Dm = OD - d;
const R = Dm / 2;
const na = nt - 2; // active coils
const pc = 1.05 * d; // dead-coil pitch
const pitch = (L - 2 * pc) / na;
if (!(na >= 1)) throw new Error(`compression spring: ${nt} coils leave no active coil (at least 3 total)`);
if (pitch <= d * 1.1) throw new Error(`compression spring: ${nt} coils of Ø${num(d)} wire do not fit ${num(L)} mm free (pitch ${pitch.toFixed(2)} ≤ wire)`);

const z = (t) => (t <= 1 ? t * pc : t <= nt - 1 ? pc + (t - 1) * pitch : pc + na * pitch + (t - nt + 1) * pc);
const path = coilPath(R, z, 0, nt, { perTurn: coilSteps(R) });
const grind = box(OD + 2, OD + 2, L).translate(-OD / 2 - 1, -OD / 2 - 1, 0);
const spring = wireAlong(path, d / 2, wireSegments(d)).intersect(grind);

// Rate, solid length and the deflection to solid (EN 13906-1).
const rate = (SPRING_G * d ** 4) / (8 * Dm ** 3 * na);
const solid = nt * d;
const massG = Math.round(7.85e-3 * spring.volume * 1000) / 1000;
const name = `${num(d)}×${num(OD)}×${num(L)}`;
const price = cents(0.18 + 0.02 * d * d * L / 10);
stock({
  partNumber: `Compression spring ${name}`,
  description: `Compression spring, music wire Ø${num(d)} mm, OD ${num(OD)} mm, free length ${num(L)} mm, ${num(nt)} coils (${num(na)} active), ends closed and ground, rate ≈ ${rate.toFixed(2)} N/mm`,
  unitPrice: price,
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("McMaster-Carr", "compression springs"),
    supplierSearch("AliExpress", `compression spring ${num(d)}mm wire ${num(OD)}mm OD ${num(L)}mm`),
    supplierSearch("Amazon", `compression spring assortment ${num(d)}mm`),
  ],
});
metric("Wire diameter", d, "mm");
metric("Outside diameter", OD, "mm");
metric("Inside diameter", OD - 2 * d, "mm");
metric("Free length", L, "mm");
metric("Active coils", na);
metric("Rate", +rate.toFixed(3), "N/mm");
metric("Solid length", +solid.toFixed(2), "mm");
metric("Load at solid", +(rate * (L - solid)).toFixed(1), "N");

connector("Bottom end", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Top end", { at: [0, 0, L], axis: [0, 0, 1] });
connector("Axis", { at: [0, 0, L / 2], axis: [0, 0, 1] });
return spring;
