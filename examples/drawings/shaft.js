// A turned output shaft, C45 steel, its axis along X: an M16×1.5 thread
// with its run-out undercut, a ⌀20 g6 bearing seat with a DIN 471
// retaining-ring groove, a shoulder, a ⌀22 k6 gear seat and a plain end;
// 1×45° chamfers at both ends and on the shoulder.
const seg = (from, to, d) => cylinder(to - from, d / 2).translate(0, 0, from);
const cone = (from, to, d0, d1) => cylinder(to - from, d0 / 2, { top: d1 / 2 }).translate(0, 0, from);
const c = p.chamfer;
const pieces = [
  cone(0, c, 16 - 2 * c, 16), seg(c, 20, 16), // M16×1.5 thread
  seg(20, 24, 13.5), // thread undercut
  seg(24, 40, 20), seg(40, 41.3, 19), seg(41.3, 44, 20), // bearing seat and ring groove
  seg(44, 54 - c, 26), cone(54 - c, 54, 26, 26 - 2 * c), // shoulder
  seg(54, 94, 22), // gear seat
  seg(94, 110 - c, 18), cone(110 - c, 110, 18, 18 - 2 * c), // plain end
];
return union(...pieces).rotate(0, 90, 0);
