// T-slot extrusion section helpers shared by tslot_30 and tslot_40 (8 mm
// slot B-type profiles), and the T-slot fittings that go with them.
//
// A section is a column of `cells` square cells of module M along Y (3030:
// one 30 mm cell, 3060: two), centred on the origin: on each cell a slot on
// ±X, a slot on each end face ±Y, the two cavities between neighbouring
// cells joined into one closed void, a core bore per cell and a hollow
// in each corner of each cell (the corner tubes of the real section).

/** One slot cavity opening on the +Y face of a cell centred on the origin. */
function tslotCavity(s) {
  const f = s.M / 2;
  return polygon([
    [-s.slot / 2, f + 0.01],
    [-s.slot / 2, f - s.lip],
    [-s.tHalf, f - s.lip],
    [-s.tHalf, f - s.tDepth],
    [-s.tipHalf, f - s.depth],
    [s.tipHalf, f - s.depth],
    [s.tHalf, f - s.tDepth],
    [s.tHalf, f - s.lip],
    [s.slot / 2, f - s.lip],
    [s.slot / 2, f + 0.01],
  ]);
}

/** The whole section (a Profile) and the cells' centres along Y. */
function tslotSection(s, cells) {
  const W = s.M;
  const D = s.M * cells;
  const centres = [];
  for (let i = 0; i < cells; i++) centres.push(-D / 2 + s.M / 2 + s.M * i);
  const cuts = [];
  const [c0, c1] = s.corner; // corner hollow from c0 to c1 from the cell centre, both ways
  const hollow = rect(c1 - c0, c1 - c0).translate(c0, c0).offset(-0.6).offset(0.6);
  for (const cy of centres) {
    cuts.push(tslotCavity(s).rotate(-90).translate(0, cy), tslotCavity(s).rotate(90).translate(0, cy));
    cuts.push(circle(s.bore / 2).translate(0, cy));
    for (const r of [0, 90, 180, 270]) cuts.push(hollow.rotate(r).translate(0, cy));
  }
  cuts.push(tslotCavity(s).translate(0, D / 2 - s.M / 2), tslotCavity(s).rotate(180).translate(0, -(D / 2 - s.M / 2)));
  for (let i = 1; i < cells; i++) {
    // Between two cells: both inner cavities joined into one closed void.
    cuts.push(
      polygon([
        [-s.tipHalf, -s.depth],
        [s.tipHalf, -s.depth],
        [s.tHalf, -s.tDepth],
        [s.tHalf, s.tDepth],
        [s.tipHalf, s.depth],
        [-s.tipHalf, s.depth],
        [-s.tHalf, s.tDepth],
        [-s.tHalf, -s.tDepth],
      ]).translate(0, -D / 2 + s.M * i),
    );
  }
  return { section: rect(W, D, { center: true }).subtract(...cuts), centres, W, D };
}

/** The usual extrusion connectors: both ends and each slot line at mid-length, facing out. */
function tslotConnectors(W, D, L, centres) {
  connector("End A", { at: [0, 0, 0], axis: [0, 0, -1] });
  connector("End B", { at: [0, 0, L], axis: [0, 0, 1] });
  centres.forEach((cy, i) => {
    const tag = centres.length === 1 ? "" : ` ${i + 1}`;
    connector(`Slot +X${tag}`, { at: [W / 2, cy, L / 2], axis: [1, 0, 0], x: [0, 0, 1] });
    connector(`Slot -X${tag}`, { at: [-W / 2, cy, L / 2], axis: [-1, 0, 0], x: [0, 0, 1] });
  });
  connector("Slot +Y", { at: [0, D / 2, L / 2], axis: [0, 1, 0], x: [0, 0, 1] });
  connector("Slot -Y", { at: [0, -D / 2, L / 2], axis: [0, -1, 0], x: [0, 0, 1] });
}
