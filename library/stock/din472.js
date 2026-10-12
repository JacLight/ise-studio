// Retaining ring for bores (internal circlip), DIN 472 regular type.
//
// Source: DIN 472:2011 table 1 — for bore d1: thickness s, free outer
// diameter d3, lug width a (max), radial width b (≈), pliers' hole d5
// (min), groove diameter d2 and groove width m (H13). The standard starts
// at an 8 mm bore. The shape is retaining_ring.js.

const STANDARD = "DIN 472";
const SHAFT = false;
const RING = {
  //      s     d3    a     b    d5    d2    m
  8: [0.8, 8.7, 2.4, 1.1, 1.0, 8.4, 0.9],
  10: [1.0, 10.8, 3.2, 1.4, 1.2, 10.4, 1.1],
  12: [1.0, 13.0, 3.4, 1.7, 1.5, 12.5, 1.1],
  15: [1.0, 16.2, 3.7, 2.0, 1.7, 15.7, 1.1],
  16: [1.0, 17.3, 3.8, 2.0, 1.7, 16.8, 1.1],
  19: [1.0, 20.5, 4.1, 2.2, 2.0, 20.0, 1.1],
  20: [1.0, 21.5, 4.1, 2.3, 2.0, 21.0, 1.1],
  22: [1.0, 23.5, 4.2, 2.5, 2.0, 23.0, 1.1],
  24: [1.2, 25.9, 4.4, 2.6, 2.0, 25.2, 1.3],
  25: [1.2, 26.9, 4.5, 2.7, 2.0, 26.2, 1.3],
  28: [1.2, 30.1, 4.8, 2.9, 2.0, 29.4, 1.3],
  30: [1.2, 32.1, 4.8, 3.0, 2.0, 31.4, 1.3],
};
// Typical price of one ring in a pack of 100 (USD).
const RING_PRICE = { 8: 0.025, 10: 0.03, 12: 0.03, 15: 0.04, 16: 0.04, 19: 0.05, 20: 0.05, 22: 0.06, 24: 0.065, 25: 0.07, 28: 0.08, 30: 0.09 };
