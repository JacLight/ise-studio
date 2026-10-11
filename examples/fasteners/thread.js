// ISO 68-1 basic metric thread, built from its definition.
//
// A thread is its tooth profile carried along a helix. In any axial plane the
// ISO profile is straight lines: 60° flanks, a P/8 crest flat at the major
// radius and a P/4 root flat at the minor radius (major − 5H/8, H = (√3/2)·P).
// So the rod is a core cylinder at the minor radius plus one helical ridge:
// the trapezoid (root → flank → crest → flank) swept around the axis while it
// rises one pitch per turn. Flanks and corners are exact; only the helix is
// approximated, by chords no farther than `tolerance` from the true curve.
//
// `phase` is the height at which a crest centre sits at angle 0, so mating
// threads can be placed exactly in phase.
function threadedRod({ majorRadius, pitch, z0, z1, phase = z0 }) {
  const H = (Math.sqrt(3) / 2) * pitch;
  const minorRadius = majorRadius - (5 * H) / 8;
  // The ridge's base reaches a little inside the core so the union is solid;
  // the flanks keep their 60° line inward, staying narrower than one pitch.
  const sink = 0.1 * pitch;
  const base = (3 * pitch) / 8 + sink * Math.tan(Math.PI / 6);
  const profile = [
    [minorRadius - sink, -base], // inner, lower
    [majorRadius, -pitch / 16], // crest, lower corner
    [majorRadius, pitch / 16], // crest, upper corner
    [minorRadius - sink, base], // inner, upper
  ];

  // The ridge must run past both ends so trimming leaves full teeth.
  const perTurn = segmentsFor(majorRadius);
  const t0 = Math.floor((z0 - phase) / pitch) - 1;
  const t1 = Math.ceil((z1 - phase) / pitch) + 1;
  const steps = (t1 - t0) * perTurn;
  const positions = [];
  for (let s = 0; s <= steps; s++) {
    const turn = t0 + s / perTurn;
    const angle = 2 * Math.PI * turn;
    const zc = phase + turn * pitch; // crest centre on the helix
    for (const [r, dz] of profile) positions.push([r * Math.cos(angle), r * Math.sin(angle), zc + dz]);
  }
  const at = (s, j) => s * 4 + j;
  const triangles = [];
  for (let s = 0; s < steps; s++) {
    for (let j = 0; j < 4; j++) {
      const k = (j + 1) % 4;
      triangles.push([at(s, j), at(s + 1, j), at(s + 1, k)], [at(s, j), at(s + 1, k), at(s, k)]);
    }
  }
  // End caps: the flat tooth profile at the first and last step.
  triangles.push([at(0, 0), at(0, 1), at(0, 2)], [at(0, 0), at(0, 2), at(0, 3)]);
  triangles.push([at(steps, 0), at(steps, 2), at(steps, 1)], [at(steps, 0), at(steps, 3), at(steps, 2)]);
  const ridge = fromMesh(positions, triangles);

  const core = cylinder(z1 - z0, minorRadius).translate(0, 0, z0);
  const slab = cylinder(z1 - z0, majorRadius + 1).translate(0, 0, z0);
  return core.add(ridge).intersect(slab);
}
