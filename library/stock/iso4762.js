// Hexagon socket head cap screw, ISO 4762 (DIN 912), stainless A2-70.
//
// Source: ISO 4762:2004 table 1 — head diameter dk (max, plain head), head
// height k (max), socket across flats s (nominal), socket depth t (min),
// under-head fillet r (min); thread length b = 2d + 12 (reference, l ≤ 125);
// ISO 261 coarse pitch. Screws whose length leaves less than about 3 pitches
// of plain shank under the head are threaded right up to it (ISO 4762's
// "thread to head" lengths).
//
// Stands with the head's bearing face on z = 0, head up +Z, shank down −Z.
// Mass: the ISO nominal geometry at 7.9 g/cm³ (stainless), as the DIN 912
// weight tables are worked out.

const ISO4762 = {
  //     dk     k    s     t    r
  2: [3.8, 2.0, 1.5, 1.0, 0.1],
  2.5: [4.5, 2.5, 2.0, 1.1, 0.1],
  3: [5.5, 3.0, 2.5, 1.3, 0.1],
  4: [7.0, 4.0, 3.0, 2.0, 0.2],
  5: [8.5, 5.0, 4.0, 2.5, 0.2],
  6: [10.0, 6.0, 5.0, 3.0, 0.25],
  8: [13.0, 8.0, 6.0, 4.0, 0.4],
};
// Typical price of one screw in a pack of 100 (stainless A2, USD): a base
// per size plus a little per millimetre of length.
const PRICE = { 2: [0.045, 0.002], 2.5: [0.045, 0.002], 3: [0.04, 0.0025], 4: [0.05, 0.003], 5: [0.06, 0.004], 6: [0.08, 0.005], 8: [0.13, 0.008] };

const d = p.d;
const L = p.length;
const [dk, k, s, t, r] = lookup(ISO4762, d, "ISO 4762");
const pitch = coarsePitch(d);
const b = 2 * d + 12;
const plain = L - b >= 3 * pitch ? L - b : 0; // plain shank under the head
const threaded = L - plain;

// Head: plain cylinder with the top edge broken (a 0.08·d bevel, where a
// cold-headed screw has its small round), the hex socket in its top face.
const c = 0.08 * d;
const head = polygon([[0, 0], [dk / 2, 0], [dk / 2, k - c], [dk / 2 - c, k], [0, k]]).revolve(360).subtract(hexSocket(s, t, k));
// Shank: plain diameter d under the head, then the thread with its 45°
// lead-in at the tip; the under-head fillet as a 45° ring of size r.
const pieces = [head, cylinder(r, d / 2, { top: d / 2 + r }).translate(0, 0, -r)];
if (plain > 0) pieces.push(cylinder(plain + 0.01, d / 2).translate(0, 0, -plain));
pieces.push(externalThread({ d, pitch, z0: -L, z1: -plain + 0.01, real: p.threads >= 0.5, tip: "bottom" }));
const screw = union(pieces);

// Mass from the nominal geometry: head less socket, plain shank at d, the
// threaded length at the pitch diameter d2 = d − 0.6495·P (equal area).
const rho = 7.9e-3; // g/mm³
const d2 = d - 0.6495 * pitch;
const socketVol = (3 * Math.sqrt(3) / 2) * (s / Math.sqrt(3)) ** 2 * t;
const massG = rho * (Math.PI / 4 * dk * dk * k - socketVol + Math.PI / 4 * (d * d * plain + d2 * d2 * threaded));
const [base, perMm] = lookup(PRICE, d, "price");
const size = `M${num(d)}×${num(L)}`;
stock({
  standard: "ISO 4762",
  partNumber: `ISO 4762 ${size} A2-70`,
  description: `Socket head cap screw ${size}, stainless A2-70, ${num(s)} mm hex key`,
  unitPrice: Math.round((base + perMm * L) * 1000) / 1000,
  currency: "USD",
  pack: 100,
  massG: Math.round(massG * 1000) / 1000,
  sources: [
    { supplier: "Misumi", sku: `SCB${num(d)}-${num(L)}`, url: `https://us.misumi-ec.com/vona2/result/?Keyword=SCB${num(d)}-${num(L)}` },
    supplierSearch("McMaster-Carr", "socket head screws"),
    supplierSearch("Bolt Depot", `M${num(d)} x ${num(L)} socket cap screw stainless`),
    supplierSearch("AliExpress", `M${num(d)} socket head cap screw A2 stainless ${num(L)}mm`),
  ],
});
metric("Hex key", s, "mm");
metric("Thread length", threaded, "mm");

// Where things attach: the head's bearing face (seat) facing down the shank,
// the screw axis, and the socket for the key.
connector("Head seat", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Axis", { at: [0, 0, -L], axis: [0, 0, 1] });
connector("Socket", { at: [0, 0, k], axis: [0, 0, 1] });
return screw;
