// Hexagon socket countersunk head screw, ISO 10642 (DIN 7991), stainless A2-70.
//
// Source: ISO 10642:2004 table 1 — head diameter dk theoretical (max) and
// actual (min), head height k (max, = (dk theoretical − d)/2 for the 90°
// head), socket across flats s (nominal), socket depth t (min); thread
// length b = 2d + 12 (reference); ISO 261 coarse pitch. The nominal length
// l is overall, head included (the head sits flush). Lengths leaving less
// than about 3 pitches of plain shank are threaded to the head.
//
// Stands with the head's top face flush at z = 0 (the surface the screw is
// sunk into), the 90° countersink cone and shank running down −Z.
// Mass: the ISO nominal geometry at 7.9 g/cm³ (stainless).

const ISO10642 = {
  //   dk theo  dk min   k     s    t
  3: [6.72, 5.54, 1.86, 2.0, 1.1],
  4: [8.96, 7.53, 2.48, 2.5, 1.5],
  5: [11.2, 9.43, 3.1, 3.0, 1.9],
  6: [13.44, 11.34, 3.72, 4.0, 2.2],
  8: [17.92, 15.24, 4.96, 5.0, 3.0],
};
// Typical price of one screw in a pack of 100 (stainless A2, USD).
const PRICE = { 3: [0.045, 0.0025], 4: [0.055, 0.003], 5: [0.065, 0.004], 6: [0.09, 0.005], 8: [0.15, 0.008] };

const d = p.d;
const L = p.length;
const [dkTheo, dkMin, k, s, t] = lookup(ISO10642, d, "ISO 10642");
const pitch = coarsePitch(d);
const b = 2 * d + 12;
const shank = L - k; // under the head
const plain = shank - b >= 3 * pitch ? shank - b : 0;
const threaded = shank - plain;

// Head: the real head stops short of the theoretical sharp edge: a 90° cone
// from d up to the actual head diameter (between min and theoretical), then
// a short cylindrical land to the top face.
const dk = (dkTheo + dkMin) / 2;
const cone = (dk - d) / 2; // 90° included: radius grows 1 per 1 of height
const head = polygon([[0, -k], [d / 2, -k], [dk / 2, -k + cone], [dk / 2, 0], [0, 0]]).revolve(360).subtract(hexSocket(s, t, 0));
const pieces = [head];
if (plain > 0) pieces.push(cylinder(plain + 0.01, d / 2).translate(0, 0, -k - plain));
pieces.push(externalThread({ d, pitch, z0: -L, z1: -k - plain + 0.01, real: p.threads >= 0.5, tip: "bottom" }));
const screw = union(pieces);

// Mass from the nominal geometry: the head (cone frustum + land) less the
// socket, plain shank at d, threaded length at d2 = d − 0.6495·P.
const rho = 7.9e-3;
const d2 = d - 0.6495 * pitch;
const frustum = (Math.PI * cone / 3) * ((d / 2) ** 2 + (d / 2) * (dk / 2) + (dk / 2) ** 2);
const land = (Math.PI / 4) * dk * dk * (k - cone);
const socketVol = (3 * Math.sqrt(3) / 2) * (s / Math.sqrt(3)) ** 2 * t;
const massG = rho * (frustum + land - socketVol + (Math.PI / 4) * (d * d * plain + d2 * d2 * threaded));
const [base, perMm] = lookup(PRICE, d, "price");
const size = `M${num(d)}×${num(L)}`;
stock({
  standard: "ISO 10642",
  partNumber: `ISO 10642 ${size} A2-70`,
  description: `Countersunk socket head screw ${size}, 90° head, stainless A2-70, ${num(s)} mm hex key`,
  unitPrice: Math.round((base + perMm * L) * 1000) / 1000,
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    supplierSearch("McMaster-Carr", "flat head socket cap screws"),
    supplierSearch("Bolt Depot", `M${num(d)} x ${num(L)} flat socket cap screw stainless`),
    supplierSearch("Misumi", `countersunk socket head screw M${num(d)} ${num(L)}`),
    supplierSearch("AliExpress", `M${num(d)} countersunk socket screw ISO 10642 A2 ${num(L)}mm`),
  ],
});
metric("Hex key", s, "mm");
metric("Countersink diameter", dkTheo, "mm");
metric("Thread length", threaded, "mm");

// The head sits flush: its top face is the surface it is sunk into; the
// countersink seat is the cone (its mouth at z = 0, facing down the shank).
connector("Head top", { at: [0, 0, 0], axis: [0, 0, 1] });
connector("Countersink seat", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Axis", { at: [0, 0, -L], axis: [0, 0, 1] });
connector("Socket", { at: [0, 0, 0], axis: [0, 0, 1] });
return screw;
