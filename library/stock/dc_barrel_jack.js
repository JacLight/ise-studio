// DC barrel power jacks for 5.5 × 2.1 mm plugs: the DC-005 through-hole
// PCB jack and the DC-022 threaded panel jack.
//
// Sources: DC-005 drawing (as sold by LCSC / Adafruit #373: 14.2 × 9.0 ×
// 11.0 mm body, Ø6.4 mm socket for the 5.5 mm plug, Ø2.0 mm centre pin
// 6.5 mm above the board, three flat 3.0 × 0.6 mm(ish) pins: centre pin at
// the back, sleeve under the socket, switch to the side); CUI PJ-002A
// shares the outline. DC-022 drawing: M8 × 0.75 bushing 9 mm long, Ø10.5
// mm stop collar, insulated body behind with three solder lugs, M8 hex
// nut (11 mm across flats, 2 mm) on the front.
//
// DC-005: the board's top face on z = 0, the socket facing −Y with the
// front face on y = 0, the body back along +Y, centred on x = 0.
// DC-022: the panel's back face on z = 0 (the collar seats on it), the
// socket opening up +Z at the bushing's end, the body below the panel.

const panel = Math.round(p.model) === 1;
let list;
let H;
if (!panel) {
  const zc = 6.5;
  const L = 14.2;
  // Body: the square housing with the round nose around the socket.
  let body = box(9.0, L - 3.0, 11.0).translate(-4.5, 3.0, 0)
    .add(cylinder(3.0 + 0.01, 4.5).rotate(-90, 0, 0).translate(0, 0, zc).intersect(box(9.0, 3.01, 11).translate(-4.5, 0, 0)))
    .add(box(9.0, 3.01, zc).translate(-4.5, 0, 0));
  body = body.subtract(cylinder(L - 1.5, 3.2).rotate(-90, 0, 0).translate(0, -0.01, zc));
  // Standoffs: the body rests on four 0.5 mm feet.
  body = body.subtract(box(7, L - 2, 0.5).translate(-3.5, 1, -0.01));
  const pin = cylinder(L - 2.5, 1.0).rotate(-90, 0, 0).translate(0, 1.5, zc);
  // Contacts: the centre pin's lug at the back, the sleeve spring under the
  // socket and the switch lug on the side, each a flat tab through the board.
  const tab = (x, y, w, t, rotZ) => box(w, t, 3.5 + 1.5).translate(-w / 2, -t / 2, -3.5).rotate(0, 0, rotZ).translate(x, y, 0);
  const lugs = union(tab(0, 13.7, 3.0, 0.6, 0), tab(0, 7.7, 3.0, 0.6, 0), tab(4.7, 10.7, 3.0, 0.6, 90));
  list = [["Body", body, "PBT (black)"], ["Centre pin", pin, "Brass, tin-plated"], ["Contacts", lugs, "Copper alloy, silver-plated"]];
  connector("Board", { at: [0, 0, 0], axis: [0, 0, -1], x: [1, 0, 0] });
  connector("Mating face", { at: [0, 0, zc], axis: [0, -1, 0], x: [1, 0, 0] });
  connector("Board edge", { at: [0, 0, 0], axis: [0, -1, 0], x: [1, 0, 0] });
  connector("Pin 1", { at: [0, 13.7, 0], axis: [0, 0, -1], x: [1, 0, 0] });
  metric("Socket height", zc, "mm");
  H = 11.0;
} else {
  // Bushing with its M8 × 0.75 thread and the stop collar, the socket bore.
  const bush = externalThread({ d: 8, pitch: 0.75, z0: 0, z1: 9, tip: "top" })
    .add(cylinder(1.5, 10.5 / 2).translate(0, 0, -1.5))
    .subtract(cylinder(14, 3.2).translate(0, 0, -4));
  const insulator = cylinder(8.5, 4.6).translate(0, 0, -10).subtract(cylinder(6.6, 3.2).translate(0, 0, -6));
  const pin = cylinder(13.5, 1.0).translate(0, 0, -6);
  const lug = (a, len) => box(2.6, 0.4, len).translate(-1.3, -0.2, -10 - len + 0.5)
    .subtract(cylinder(1, 0.6).rotate(90, 0, 0).translate(0, 0.5, -10 - len + 1.6)).translate(0, 3.0, 0).rotate(0, 0, a);
  const lugs = union(lug(0, 6), lug(120, 5), lug(240, 5));
  const nut = hexPrism(11, 2).subtract(cylinder(3, 4.05).translate(0, 0, -0.5)).translate(0, 0, 4);
  list = [["Bushing", bush, "Brass, tin-plated"], ["Insulator", insulator, "PBT (black)"], ["Centre pin", pin, "Brass, tin-plated"], ["Lugs", lugs, "Brass, tin-plated"], ["Nut", nut, "Brass, tin-plated"]];
  connector("Panel", { at: [0, 0, 0], axis: [0, 0, 1], x: [1, 0, 0] });
  connector("Mating face", { at: [0, 0, 9], axis: [0, 0, 1], x: [1, 0, 0] });
  metric("Panel hole", 8.2, "mm");
  metric("Max panel", 4, "mm");
  H = 9;
}
const { parts, massG } = weighed(list);
const model = panel ? "DC-022" : "DC-005";
stock({
  partNumber: `${model} 5.5×2.1`,
  description: panel
    ? "DC barrel jack, panel mount, M8 × 0.75 bushing, for 5.5 × 2.1 mm plugs, solder lugs (DC-022)"
    : "DC barrel jack, PCB through-hole, for 5.5 × 2.1 mm plugs, 14.2 × 9 × 11 mm (DC-005)",
  mpn: model,
  unitPrice: panel ? 0.45 : 0.12,
  currency: "USD",
  pack: 1,
  massG,
  sources: [
    supplierSearch("LCSC", `${model} DC power jack 5.5x2.1`),
    supplierSearch("Adafruit", panel ? "panel mount 2.1mm DC barrel jack" : "breadboard-friendly 2.1mm DC barrel jack"),
    supplierSearch("AliExpress", `${model} 5.5x2.1mm DC jack`),
  ],
});
metric("Plug", 5.5, "mm");
metric("Centre pin", 2.1, "mm");
return parts;
