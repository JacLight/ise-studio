ISE S3 Handheld — revision A
Made with ISE Studio. Gerber RS-274X with X2 attributes, Excellon drill, millimetres.

Board: 50.00 × 72.00 mm, 4 copper layers, 1.6 mm FR4, 1 oz outer copper
Finish: ENIG   Solder mask: green   Silkscreen: white
Smallest track 0.100 mm, smallest gap 0.142 mm, smallest plated hole 0.200 mm, smallest annular ring 0.150 mm
95 components (60 top, 31 bottom), 444 holes, 380 vias; design-rule check: 0 errors
Quantity: 5

Files:
  ISE_S3_Handheld-F_Cu.gtl / -B_Cu.gbl     top / bottom copper
  ISE_S3_Handheld-In1_Cu.g2l / -In2_Cu.g3l  inner copper (layers 2 and 3)
  -F_Mask.gts / -B_Mask.gbs        solder mask openings
  -F_Paste.gtp / -B_Paste.gbp      paste stencil
  -F_Silkscreen.gto / -B_Silkscreen.gbo  legend
  -Edge_Cuts.gm1                   board outline (profile)
  -PTH.drl / -NPTH.drl             plated / unplated holes (slots as G85)
  bom.csv, positions.csv           assembly (coordinates from the board's lower-left corner)

Prepared for JLCPCB: Zip of Gerber RS-274X (Protel extensions) and Excellon drill in mm; for assembly, bom.csv (Comment, Designator, Footprint, LCSC Part #) and positions.csv (Designator, Mid X, Mid Y, Layer, Rotation)

Placement rotations are counter-clockwise from the top, from each footprint's IPC-7351 zero orientation; check them in the fab's assembly preview (their libraries' zero orientations differ for some packages).

IMPEDANCE CONTROL

Build to the stackup below and hold these impedances; adjust track widths (and pair gaps) by up to ±10 % to meet them, and report the as-built values. Test coupons per IPC-2141 / IPC-6012.

Stackup (top down):
  Top silkscreen      0.0000 mm  white ink
  Top mask            0.0100 mm  green solder mask
  top                 0.0350 mm  Copper  1 oz
  Dielectric 1        0.2104 mm  7628 prepreg  εr 4.4
  in1                 0.0175 mm  Copper  0.5 oz
  Dielectric 2        1.0650 mm  FR4 core  εr 4.6
  in2                 0.0175 mm  Copper  0.5 oz
  Dielectric 3        0.2104 mm  7628 prepreg  εr 4.4
  bottom              0.0350 mm  Copper  1 oz
  Bottom mask         0.0100 mm  green solder mask
  Bottom silkscreen   0.0000 mm  white ink

Controlled lines:
  USB        top     differential    90 Ω ±10 %  width 0.268 mm, gap 0.150 mm  (microstrip over in1; computed 90 Ω)  nets: USB_D+, USB_D-

Widths from closed-form models (Hammerstad–Jensen microstrip, Cohn/Wheeler stripline, IPC-2141 coupling), solder mask not counted; the fab's field solver is final.

