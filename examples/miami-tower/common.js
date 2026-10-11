// Shared geometry for the Miami condo + hotel tower. All values in mm.

/** Rectangle with rounded corners, centred on the origin. */
function roundedRect(w, d, r) {
  const rr = Math.max(1, Math.min(r, w / 2 - 1, d / 2 - 1));
  return rect(w - 2 * rr, d - 2 * rr, { center: true }).offset(rr, { join: "round" });
}

/**
 * The storey stack, bottom to top: double-height lobby, parking podium,
 * amenity deck (on the podium roof), hotel floors, then residences.
 * Every program derives its levels from the same parameters, so structure,
 * facade and plans always agree.
 */
function stack(p) {
  const levels = [];
  let z = 0;
  const add = (kind, label, h) => {
    levels.push({ n: levels.length + 1, kind, label, z, h });
    z += h;
  };
  add("lobby", "Lobby", p.lobbyHeight);
  for (let i = 1; i < p.podiumFloors; i++) add("parking", "Parking", p.podiumFloorHeight);
  add("amenity", "Amenity deck", p.amenityHeight);
  for (let i = 0; i < p.hotelFloors; i++) add("hotel", "Hotel", p.hotelFloorHeight);
  for (let i = 0; i < p.condoFloors; i++) add("condo", "Residences", p.condoFloorHeight);
  return { levels, roof: z };
}

const towerOutline = (p) => roundedRect(p.towerWidth, p.towerDepth, p.cornerRadius);
const podiumOutline = (p) => roundedRect(p.podiumWidth, p.podiumDepth, p.cornerRadius);
/** Residences: the slab runs out over deep balconies along both long faces. */
const condoOutline = (p) =>
  towerOutline(p).add(roundedRect(p.towerWidth - 2 * p.cornerRadius, p.towerDepth + 2 * p.balconyDepth, p.balconyDepth));
/** The tower sits toward the back of the podium, leaving the pool deck in front. */
const towerShift = (p) => (p.podiumDepth - p.towerDepth) / 2 - p.towerSetback;
