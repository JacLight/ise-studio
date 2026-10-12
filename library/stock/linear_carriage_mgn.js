// MGN miniature linear guide block (carriage): standard (MGN…C) or long
// (MGN…H), for MGN7/9/12/15 rails.
//
// Source: HIWIN MG series block table (see mgn.js): height H and H1 over
// the rail base, width W, mounting holes B × C tapped M × l, steel body
// length L1, overall length L with the end caps and seals, catalogue mass.
//
// Same frame as the rail (mgn.js): centred on x = 0, the rail's base on
// z = 0, so the block's underside is at z = H1 and its mounting face at
// z = H. Parts: the hardened steel body (rail channel, ball grooves, return
// holes, four tapped holes, side grooves), the two plastic end caps (ball
// turnarounds) and the retainer strips under its legs, the two rubber end
// seals and the balls of the two loaded rows and their return rows. "Slide" joins the rail's "Slide" (a slider joint along the
// rail; its value is where the block's centre is).

const s = mgn(p.series);
const long = p.long >= 0.5;
const k = long ? 1 : 0;
const C = s.C[k];
const L1 = s.L1[k];
const L = s.L[k];
const { yb, zb, rg } = mgnBallRows(s);
const gap = 0.1 * s.Dw; // between rail sides and the channel walls
const top = s.HR + 0.3; // channel ceiling: 0.3 mm over the rail
const c = 0.4; // body edge chamfers
const tSeal = Math.min(1.5, 0.3 * (L - L1) / 2);
const tCap = (L - L1) / 2 - tSeal;

// Cross-section in (y, z): outside chamfered, the rail channel underneath.
const outside = polygon([
  [-s.W / 2 + c, s.H1],
  [s.W / 2 - c, s.H1],
  [s.W / 2, s.H1 + c],
  [s.W / 2, s.H - c],
  [s.W / 2 - c, s.H],
  [-s.W / 2 + c, s.H],
  [-s.W / 2, s.H - c],
  [-s.W / 2, s.H1 + c],
]);
const channel = (g) => rect(s.WR + 2 * g, top - s.H1 + 0.01, { center: false }).translate(-(s.WR / 2 + g), s.H1 - 0.01);
// Return holes: one each side, mid-leg, at the ball rows' height.
const yr = (s.WR / 2 + gap + s.W / 2) / 2;
const rr = s.Dw / 2 + 0.15;
// Under each leg of the steel body a plastic retainer strip (it holds the
// balls in when the block is off the rail; MGN7 holds them with a wire
// instead); a shallow groove runs along each side face.
const tStrip = s.WR > 7 ? 0.1 * (s.H - s.H1) : 0;
const under = tStrip > 0 ? [rect(s.W + 2, tStrip + 0.01).translate(-s.W / 2 - 1, s.H1 - 0.01)] : [];
const zg = s.H1 + 0.45 * (s.H - s.H1);
const hg = 0.3 * (s.H - s.H1);
const dg = 0.03 * s.W; // side groove depth
const sideGroove = (side) => rect(2 * dg, hg).translate(side > 0 ? s.W / 2 - dg : -s.W / 2 - dg, zg - hg / 2);
const bodySection = outside.subtract(channel(gap), ...under, sideGroove(1), sideGroove(-1), circle(rg).translate(yb, zb), circle(rg).translate(-yb, zb), circle(rr).translate(yr, zb), circle(rr).translate(-yr, zb));
let body = alongX(bodySection.extrude(L1)).translate(-L1 / 2, 0, 0);
const strips = tStrip > 0 && alongX(outside.intersect(under[0]).subtract(channel(gap + 0.1)).extrude(L1)).translate(-L1 / 2, 0, 0);

// Four tapped mounting holes in the top, M × l.
const pitch = coarsePitch(s.tap);
const tap = internalThreadTool({ d: s.tap, pitch, z0: s.H - s.l, z1: s.H });
const holes = [[C / 2, s.B / 2], [-C / 2, s.B / 2], [-C / 2, -s.B / 2], [C / 2, -s.B / 2]];
body = body.subtract(union(holes.map(([x, y]) => tap.translate(x, y, 0))));

// End caps (ball turnarounds) and seals: the block's outline round the rail.
const capSection = outside.subtract(channel(gap + 0.1));
const sealSection = outside.subtract(channel(0.05));
const cap = alongX(capSection.extrude(tCap));
const seal = alongX(sealSection.extrude(tSeal));
const caps = cap.translate(L1 / 2, 0, 0).add(cap.translate(-L1 / 2 - tCap, 0, 0));
const seals = seal.translate(L1 / 2 + tCap, 0, 0).add(seal.translate(-L / 2, 0, 0));

// Balls: the loaded rows in the grooves and the return rows, along the body.
const nb = Math.floor(L1 / s.Dw) - 1;
const balls = [];
for (const [y, z] of [[yb, zb], [-yb, zb], [yr, zb], [-yr, zb]]) {
  for (let i = 0; i < nb; i++) balls.push(sphere(s.Dw / 2 - 0.01).translate(-((nb - 1) * s.Dw) / 2 + i * s.Dw, y, z));
}

const name = `MGN${s.WR}${long ? "H" : "C"}`;
stock({
  standard: "HIWIN MGN (MG series)",
  partNumber: name,
  description: `Miniature linear guide block ${name}${long ? " (long)" : ""} for MGN${s.WR} rail, ${num(s.W)}×${num(L)} mm, 4× M${s.tap} on ${num(s.B)}×${num(C)}`,
  manufacturer: "HIWIN (or MGN-pattern equivalent)",
  mpn: `${name}1`,
  unitPrice: { 7: 9, 9: 9, 12: 10, 15: 13 }[s.WR] + (long ? 2 : 0),
  currency: "USD",
  pack: 1,
  massG: s.mass[k],
  sources: [
    supplierSearch("Misumi", `miniature linear guide block ${s.WR}mm`),
    supplierSearch("McMaster-Carr", "ball bearing carriages and guide rails"),
    supplierSearch("AliExpress", `${name} carriage block`),
    supplierSearch("Amazon", `${name} linear block`),
  ],
});
metric("Block width", s.W, "mm");
metric("Block length", L, "mm");
metric("Height", s.H, "mm");
metric("Hole spacing B", s.B, "mm");
metric("Hole spacing C", C, "mm");
metric("Tapped thread", s.tap, "mm");
metric("Tapped depth", s.l, "mm");

connector("Slide", { at: [0, 0, 0], axis: [-1, 0, 0], x: [0, 0, 1] });
connector("Top", { at: [0, 0, s.H], axis: [0, 0, 1] });
connector("Bottom", { at: [0, 0, s.H1], axis: [0, 0, -1] });
holes.forEach(([x, y], i) => connector(`Hole ${i + 1}`, { at: [x, y, s.H], axis: [0, 0, 1] }));

return [
  part("Block", body, { material: "Steel S55C, hardened" }),
  part("End caps", caps, { material: "Acetal (POM)" }),
  part("End seals", seals, { material: "Nitrile rubber (NBR)" }),
  part("Balls", union(balls), { material: "Bearing steel 52100" }),
  ...(strips ? [part("Retainer strips", strips, { material: "Acetal (POM)" })] : []),
];
