// The tower as it is built: every storey's slab, columns, core lift and
// curtain wall is its own part, so each pour and each glazing package can be
// scheduled, quantified and checked. Parts meet face to face, never overlap:
// columns and core run between slabs, slabs are cut around the core.
const { levels, roof } = stack(p);
const ty = towerShift(p);
const parts = [];
const concrete = { material: "Concrete", process: "cast" };
const glazing = { material: "Glass", process: "glazing" };
const tag = (L) => `L${String(L.n).padStart(2, "0")}`;
const isPodium = (L) => L.kind === "lobby" || L.kind === "parking";

// Core plan: one elevator bank between two stairs.
const cw = p.coreWidth, cd = p.coreDepth, t = p.coreWall;
const stairW = (cw - 4 * t) * 0.25;
const liftW = cw - 4 * t - 2 * stairW;
const coreOutline = rect(cw, cd, { center: true }).translate(0, ty);
const coreRooms = [
  rect(stairW, cd - 2 * t).translate(-cw / 2 + t, -cd / 2 + t),
  rect(liftW, cd - 2 * t).translate(-cw / 2 + 2 * t + stairW, -cd / 2 + t),
  rect(stairW, cd - 2 * t).translate(cw / 2 - t - stairW, -cd / 2 + t),
].map((r) => r.translate(0, ty));
const corePlan = coreOutline.subtract(...coreRooms);
const doorX = [-cw / 2 + t + stairW / 2, 0, cw / 2 - t - stairW / 2];

// Column grids. Tower: perimeter columns on the bay grid. Podium: a parking
// grid, skipping bays the tower columns or the core already occupy.
const inset = p.columnSize / 2 + 300;
const towerCols = [];
for (let x = -p.towerWidth / 2 + p.cornerRadius; x <= p.towerWidth / 2 - p.cornerRadius + 1; x += p.bay) {
  for (const y of [-p.towerDepth / 2 + inset, p.towerDepth / 2 - inset]) towerCols.push([x, ty + y]);
}
const clearOf = (x, y, margin) =>
  !(Math.abs(x) < p.towerWidth / 2 + margin && Math.abs(y - ty) < p.towerDepth / 2 + margin);
const podiumCols = [];
for (let x = -p.podiumWidth / 2 + p.bay / 2; x < p.podiumWidth / 2; x += p.bay) {
  for (let y = -p.podiumDepth / 2 + p.bay / 2; y < p.podiumDepth / 2; y += p.bay) {
    if (clearOf(x, y, p.columnSize)) podiumCols.push([x, y]);
  }
}
const columns = (pts, z0, h) => union(pts.map(([x, y]) => box(p.columnSize, p.columnSize, h).translate(x - p.columnSize / 2, y - p.columnSize / 2, z0)));

const ring = (outline, from, thickness) => outline.offset(-from).subtract(outline.offset(-from - thickness));

for (const L of levels) {
  const top = L.z + p.slab; // top of this level's slab
  const clear = L.h - p.slab; // floor to underside of the next slab

  let outline;
  if (isPodium(L) || L.kind === "amenity") outline = podiumOutline(p);
  else if (L.kind === "condo") outline = condoOutline(p).translate(0, ty);
  else outline = towerOutline(p).translate(0, ty);
  parts.push(part(`${tag(L)} slab · ${L.label}`, outline.subtract(coreOutline).extrude(p.slab).translate(0, 0, L.z), concrete));

  const cols = isPodium(L) ? towerCols.concat(podiumCols) : towerCols;
  parts.push(part(`${tag(L)} columns`, columns(cols, top, clear), concrete));

  // One core lift per storey, with a door from every shaft onto the floor.
  const doors = doorX.map((x) => box(1200, t * 3, 2400).translate(x - 600, ty - cd / 2 - t * 1.5, top));
  parts.push(part(`${tag(L)} core`, corePlan.extrude(clear).translate(0, 0, top).subtract(union(doors)), concrete));

  if (L.kind === "lobby") {
    parts.push(part(`${tag(L)} storefront`, ring(podiumOutline(p), p.glassInset, p.glassThickness).extrude(clear).translate(0, 0, top), glazing));
  } else if (!isPodium(L)) {
    parts.push(part(`${tag(L)} curtain wall`, ring(towerOutline(p).translate(0, ty), p.glassInset, p.glassThickness).extrude(clear).translate(0, 0, top), glazing));
  }
  if (L.kind === "condo") {
    // Frameless balustrade at the balcony edge, 42″ above the slab (Miami-Dade).
    parts.push(part(`${tag(L)} balustrade`, ring(condoOutline(p).translate(0, ty), 60, 20).subtract(towerOutline(p).translate(0, ty)).extrude(1067).translate(0, 0, top), glazing));
  }
}

// Roof: slab, parapet crown, and the core rising to the machine room roof.
parts.push(part("Roof slab", towerOutline(p).translate(0, ty).subtract(coreOutline).extrude(p.slab).translate(0, 0, roof), concrete));
const crown = towerOutline(p).subtract(towerOutline(p).offset(-p.crownThickness)).translate(0, ty);
parts.push(part("Crown parapet", crown.extrude(p.crownHeight).translate(0, 0, roof + p.slab), concrete));
parts.push(part("Machine room core", coreOutline.extrude(p.crownHeight).translate(0, 0, roof + p.slab), concrete));

// Pool on the amenity deck, in front of the tower: a basin and its water.
const deck = levels.find((L) => L.kind === "amenity");
const poolY = -p.podiumDepth / 2 + p.poolDepth / 2 + 6000;
const basin = roundedRect(p.poolLength + 600, p.poolDepth + 600, 1800);
const water = roundedRect(p.poolLength, p.poolDepth, 1500);
parts.push(part("Pool basin", basin.subtract(water).extrude(1400).add(basin.extrude(200)).translate(0, poolY, deck.z + p.slab), concrete));
parts.push(part("Pool water", water.extrude(1200).translate(0, poolY, deck.z + p.slab + 200), { material: "Pool water", process: "buy" }));

return parts;
