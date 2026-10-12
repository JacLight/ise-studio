// Retaining ring for shafts (external circlip), DIN 471 regular type.
//
// Source: DIN 471:2011 table 1 — for shaft d1: thickness s, free inner
// diameter d3, lug width a (max), radial width b (≈), pliers' hole d5
// (min), groove diameter d2 and groove width m (H13). The shape is
// retaining_ring.js.

const STANDARD = "DIN 471";
const SHAFT = true;
const RING = {
  //      s     d3    a     b    d5    d2    m
  4: [0.4, 3.7, 2.2, 0.9, 1.0, 3.8, 0.5],
  5: [0.6, 4.7, 2.5, 1.1, 1.0, 4.8, 0.7],
  6: [0.7, 5.6, 2.7, 1.3, 1.2, 5.7, 0.8],
  8: [0.8, 7.4, 3.2, 1.5, 1.2, 7.6, 0.9],
  10: [1.0, 9.3, 3.3, 1.8, 1.5, 9.6, 1.1],
  12: [1.0, 11.0, 3.3, 1.8, 1.7, 11.5, 1.1],
  15: [1.0, 13.8, 3.6, 2.2, 1.7, 14.3, 1.1],
  16: [1.0, 14.7, 3.7, 2.2, 1.7, 15.2, 1.1],
  17: [1.0, 15.7, 3.8, 2.3, 1.7, 16.2, 1.1],
  20: [1.2, 18.5, 4.0, 2.6, 2.0, 19.0, 1.3],
  22: [1.2, 20.5, 4.2, 2.8, 2.0, 21.0, 1.3],
  25: [1.2, 23.2, 4.4, 3.0, 2.0, 23.9, 1.3],
  30: [1.5, 27.9, 5.0, 3.5, 2.0, 28.6, 1.6],
};
// Typical price of one ring in a pack of 100 (USD).
const RING_PRICE = { 4: 0.02, 5: 0.02, 6: 0.02, 8: 0.025, 10: 0.03, 12: 0.03, 15: 0.04, 16: 0.04, 17: 0.045, 20: 0.05, 22: 0.06, 25: 0.07, 30: 0.09 };
