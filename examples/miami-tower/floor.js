// One storey of the tower, as a component: its slab, columns, core lift,
// façade and fit-out, from its own slab (z = 0) up to the next one (z = height).
// The tower places one instance per storey; a sold or customised storey is
// made unique and changed on its own.
//
// kind:   0 lobby · 1 parking · 2 amenity deck (with the pool) · 3 hotel · 4 residence · 5 roof
// layout: 0 shell (unsold) · 1 full-floor residence · 2 hotel-style suites
// stairUp / stairDown: an internal stair to the storey above / an opening for
// the one from below, so consecutive storeys bought together become one home.
const K = p.kind, h = p.height, slab = p.slab, clear = h - slab;
const ty = towerShift(p);
const parts = [];
const concrete = { material: "Concrete", process: "cast" };
const glazing = { material: "Glass", process: "glazing" };
const drywall = { material: "Gypsum", process: "buy" };

// Core: one elevator bank between two stairs.
const cw = p.coreWidth, cd = p.coreDepth, t = p.coreWall;
const stairW = (cw - 4 * t) * 0.25;
const liftW = cw - 4 * t - 2 * stairW;
const coreOutline = rect(cw, cd, { center: true }).translate(0, ty);
const corePlan = coreOutline.subtract(
  ...[
    rect(stairW, cd - 2 * t).translate(-cw / 2 + t, -cd / 2 + t),
    rect(liftW, cd - 2 * t).translate(-cw / 2 + 2 * t + stairW, -cd / 2 + t),
    rect(stairW, cd - 2 * t).translate(cw / 2 - t - stairW, -cd / 2 + t),
  ].map((r) => r.translate(0, ty)),
);
const doorX = [-cw / 2 + t + stairW / 2, 0, cw / 2 - t - stairW / 2];

// The internal stair, in the front zone clear of columns, core and suites.
const risers = Math.ceil(h / 185);
const rise = h / risers, going = 280, stairWidth = 1100;
const sx = -p.towerWidth * 0.3, sy = ty - p.towerDepth / 2 + 2600;
const opening = rect(risers * going + 600, stairWidth + 400).translate(sx - 300, sy - 200);

const ring = (outline, from, thickness) => outline.offset(-from).subtract(outline.offset(-from - thickness));

if (K === 5) {
  // Roof: slab, parapet crown and the core rising to the machine room.
  parts.push(part("Roof slab", towerOutline(p).translate(0, ty).subtract(coreOutline).extrude(slab), concrete));
  const crown = towerOutline(p).subtract(towerOutline(p).offset(-p.crownThickness)).translate(0, ty);
  parts.push(part("Crown parapet", crown.extrude(p.crownHeight).translate(0, 0, slab), concrete));
  parts.push(part("Machine room", coreOutline.extrude(p.crownHeight).translate(0, 0, slab), concrete));
  return parts;
}

// Slab, cut around the core (and for a stair coming up from below).
let outline = K <= 2 ? podiumOutline(p) : K === 4 ? condoOutline(p).translate(0, ty) : towerOutline(p).translate(0, ty);
let floor = outline.subtract(coreOutline);
if (p.stairDown) floor = floor.subtract(opening);
parts.push(part("Slab", floor.extrude(slab), concrete));
if (K >= 3) metric("Net area", floor.area / 1e6, "m²");

// Columns: perimeter grid; the podium adds a parking grid outside the tower.
const inset = p.columnSize / 2 + 300;
const cols = [];
for (let x = -p.towerWidth / 2 + p.cornerRadius; x <= p.towerWidth / 2 - p.cornerRadius + 1; x += p.bay) {
  for (const y of [-p.towerDepth / 2 + inset, p.towerDepth / 2 - inset]) cols.push([x, ty + y]);
}
if (K <= 1) {
  for (let x = -p.podiumWidth / 2 + p.bay / 2; x < p.podiumWidth / 2; x += p.bay) {
    for (let y = -p.podiumDepth / 2 + p.bay / 2; y < p.podiumDepth / 2; y += p.bay) {
      if (!(Math.abs(x) < p.towerWidth / 2 + p.columnSize && Math.abs(y - ty) < p.towerDepth / 2 + p.columnSize)) cols.push([x, y]);
    }
  }
}
parts.push(part("Columns", union(cols.map(([x, y]) => box(p.columnSize, p.columnSize, clear).translate(x - p.columnSize / 2, y - p.columnSize / 2, slab))), concrete));

// Core lift, a door from every shaft onto the floor.
const doors = doorX.map((x) => box(1200, t * 3, 2400).translate(x - 600, ty - cd / 2 - t * 1.5, slab));
parts.push(part("Core", corePlan.extrude(clear).translate(0, 0, slab).subtract(union(doors)), concrete));

// Façade.
if (K === 0) parts.push(part("Storefront", ring(podiumOutline(p), p.glassInset, p.glassThickness).extrude(clear).translate(0, 0, slab), glazing));
if (K >= 2) parts.push(part("Curtain wall", ring(towerOutline(p).translate(0, ty), p.glassInset, p.glassThickness).extrude(clear).translate(0, 0, slab), glazing));
if (K === 4) {
  // Frameless balustrade at the balcony edge, 42″ above the slab (Miami-Dade).
  const rail = ring(condoOutline(p).translate(0, ty), 60, 20).subtract(towerOutline(p).translate(0, ty)).extrude(1067).translate(0, 0, slab);
  parts.push(part("Balustrade", rail, glazing));
}

// Pool on the amenity deck, in front of the tower: basin and water.
if (K === 2) {
  const poolY = -p.podiumDepth / 2 + p.poolDepth / 2 + 6000;
  const basin = roundedRect(p.poolLength + 600, p.poolDepth + 600, 1800);
  const water = roundedRect(p.poolLength, p.poolDepth, 1500);
  parts.push(part("Pool basin", basin.subtract(water).extrude(1400).add(basin.extrude(200)).translate(0, poolY, slab), concrete));
  parts.push(part("Pool water", water.extrude(1200).translate(0, poolY, slab + 200), { material: "Pool water", process: "buy" }));
}

// Fit-out. Walls stop short of the glass and stand between the columns.
const wallT = 125;
const glassFront = ty - p.towerDepth / 2 + p.glassInset + p.glassThickness + 20;
const glassBack = ty + p.towerDepth / 2 - p.glassInset - p.glassThickness - 20;
// Mid-bay (clear of the perimeter columns), and only where the façade runs
// straight: a narrower tower gets fewer suites, not walls through its corners.
const straight = p.towerWidth / 2 - p.cornerRadius - 300;
const wallXs = [];
for (let k = -Math.floor(straight / p.bay); k * p.bay <= straight; k++) wallXs.push(k * p.bay);
const wall = (x0, y0, x1, y1) => box(Math.max(x1 - x0, wallT), Math.max(y1 - y0, wallT), clear).translate(x0, y0, slab);
if (p.layout === 1) {
  // Full-floor residence: bedroom walls on the private (back) side, open living to the front.
  const from = ty + cd / 2 + 1000; // a passage along the core
  const bedrooms = wallXs.filter((x) => x !== 0 && Math.abs(x) <= 2 * p.bay);
  bedrooms.forEach((x, i) => parts.push(part(`Bedroom wall ${i + 1}`, wall(x - wallT / 2, from, x + wallT / 2, glassBack), drywall)));
  if (bedrooms.length) {
    const reach = Math.max(...bedrooms.map(Math.abs));
    parts.push(part("Gallery wall", wall(-reach, from - wallT, reach, from), drywall));
  }
  metric("Homes", 1);
}
if (p.layout === 2) {
  // Hotel-style suites both sides of a corridor around the core.
  const corridor = 1800;
  const front = ty - cd / 2 - corridor, back = ty + cd / 2 + corridor;
  const ends = [-p.towerWidth / 2 + 3000, p.towerWidth / 2 - 3000];
  if (!wallXs.length) throw new Error("the tower is too narrow for suites at this column bay");
  const bounds = [ends[0], ...wallXs, ends[1]];
  let n = 0;
  for (const side of [-1, 1]) {
    const yWall = side < 0 ? front - wallT : back;
    // Corridor wall with one door per suite.
    let corridorWall = wall(ends[0], yWall, ends[1], yWall + wallT);
    for (let i = 0; i < bounds.length - 1; i++) {
      const cx = (bounds[i] + bounds[i + 1]) / 2;
      corridorWall = corridorWall.subtract(box(900, wallT * 3, 2100).translate(cx - 450, yWall - wallT, slab));
      n++;
    }
    parts.push(part(`Corridor wall ${side < 0 ? "front" : "back"}`, corridorWall, drywall));
    wallXs.forEach((x, i) => {
      const [y0, y1] = side < 0 ? [glassFront, yWall] : [yWall + wallT, glassBack];
      parts.push(part(`Demising wall ${side < 0 ? "F" : "B"}${i + 1}`, wall(x - wallT / 2, y0, x + wallT / 2, y1), drywall));
    });
  }
  metric("Keys", n);
}

// Internal stair up to the next storey: a precast flight landing in that storey's opening.
if (p.stairUp) {
  const steps = [];
  for (let i = 0; i < risers; i++) {
    const z0 = i === 0 ? slab : slab + i * rise - 150;
    steps.push(box(going, stairWidth, slab + (i + 1) * rise - z0).translate(sx + i * going, sy, z0));
  }
  parts.push(part("Internal stair", union(steps), concrete));
}

return parts;
