# Renders and 3D for the web

Iṣẹ́ Studio makes photoreal product images and interactive 3D models for websites straight from the design: the same model the engineering checks run on, so the pictures always match the product.

## Photoreal renders

Open the **View** panel and press **Render**: the view is path-traced live (three-gpu-pathtracer), with true metals, plastics, glass and soft shadows. Choose **White sweep** for catalogue shots (a pure white backdrop and floor, the product's soft shadow) or the grey **Studio**, then **Save PNG** at the size and quality you want.

From the command line, set the view first, then render:

```sh
node tools/ise.mjs view isolate "PhaZZer Enforcer" --project phazzer-enforcer-3d
node tools/ise.mjs view camera 145,22,26,0.82 --project phazzer-enforcer-3d   # azimuth, elevation, lens °, fill
node tools/ise.mjs render --width 2000 --height 2000 --samples 512 --studio white --out hero.png --project phazzer-enforcer-3d
```

- **The product studio** (white sweep) lights like a product photographer and turns with the camera: a low key light raking across the sides so slots, grooves and ribs cast crisp shadows, faint fill, bright rim strips behind tracing the edges, black cards round the camera so gloss and chrome read as themselves.
- **Lens**: straight side, front and top views read best with a narrow lens (`…,10,…`), as in catalogue photos; three-quarter hero shots with 22–26°.
- **`--headless`** renders in a browser of its own, so a background tab (which browsers slow down) cannot hold a render up. With no app tab open, a headless browser is used anyway.
- **`--timeout <minutes>`** stops waiting after that long.

### Materials from their names

A part looks the way its material's name says, so naming it is styling it:

| In the name | Looks like |
|---|---|
| `ABS`, `PC/ABS`, `POM`, `TPE`, `rubber`, `epoxy` | that plastic |
| `steel`, `stainless`, `aluminium`, `brass`, `copper`, `chrome` | that metal (chrome: a mirror plate) |
| `#f8c600`, `yellow`, `matte black` | that colour |
| `polished`, `gloss`, `satin`, `matte`, `brushed`, `bead blasted` | that finish |
| `textured`, `grained`, `mould texture` | the fine grain a mould gives a matte housing |
| `stippled`, `pebbled` | a rubber grip's pebbles |
| `brushed`, `hairline` (on a metal) | the abrasive's fine lines |
| `clear`, `translucent` | see-through (coloured ones glow their colour) |
| `print <picture>` | a label or logo printed from a picture added to the project |

Example: `ABS (yellow #f8c600, matte textured)`, `Epoxy dome (black #0d0d0d, gloss)`, `Stainless steel 304 brushed`.

### Photo skins

When a product was modelled from calibrated reference photos (canvases), `--skin "Left side,Top"` lays those photos back onto the faces they saw, so every printed mark and texture is exactly as photographed. It works best where the model's shape matches the photo closely; anything modelled in the wrong place shows twice.

## 3D for websites

**Save 3D for web** (in render mode), or from the command line:

```sh
node tools/ise.mjs render --glb enforcer.glb --project phazzer-enforcer-3d
```

writes what the view shows as a binary glTF (`.glb`): the render's materials and printed labels, in metres with Y up, standing on the ground, ready for `<model-viewer>`, Shopify, WooCommerce and AR on phones. Each model also carries its **ambient occlusion baked in** (as vertex colours), so slots, grooves and openings keep their depth in any web viewer, which casts no shadow into them by itself.

Make it small for the web (Draco geometry, WebP textures):

```sh
npx @gltf-transform/cli@4.1.1 optimize enforcer.glb enforcer-web.glb --compress draco --texture-compress webp --simplify false
```

### On a web page

Make the studio lighting file once (`python3 tools/studio-hdr.py studio.hdr`, it needs numpy) and upload it next to the models, then:

```html
<script type="module" src="https://cdn.jsdelivr.net/npm/@google/model-viewer@4.0.0/dist/model-viewer.min.js"></script>

<model-viewer src="enforcer-yellow.glb" poster="enforcer-yellow.jpg" alt="PhaZZer Enforcer, yellow"
  camera-controls auto-rotate ar ar-modes="webxr scene-viewer quick-look"
  environment-image="studio.hdr" tone-mapping="neutral" exposure="1.45"
  shadow-intensity="1" shadow-softness="0.5" camera-orbit="35deg 72deg auto"
  style="width:100%; height:480px; background:#fff"></model-viewer>
```

`studio.hdr` lights the model the way the renders do (a key light from the upper left, rim lights, dark sides), instead of the viewer's flat default. AR on phones needs the page served over https.
