"""studio-hdr.py <out.hdr>: a product-studio lighting file for the website's 3D viewer (Radiance .hdr, equirectangular,
Y up): a strong key softbox high on the default camera's left, faint fill, bright rim strips behind, dark sides, so
mouldings, slots and edges read with depth when the model turns (as the renders' product studio does)."""
import sys, math
import numpy as np
W, H = 1024, 512
# Directions (three.js equirect): u = atan2(z, x)/2π + 0.5, v = acos(y)/π. Azimuth here: 0 toward +Z (where the
# viewer's camera starts, turned 35° toward +X), +90 toward +X; elevation up from the horizon.
u = (np.arange(W) + 0.5) / W
v = (np.arange(H) + 0.5) / H
U, V = np.meshgrid(u, v)
phi = (U - 0.5) * 2 * math.pi          # atan2(z, x)
theta = V * math.pi                    # from +Y
x, y, z = np.sin(theta) * np.cos(phi), np.cos(theta), np.sin(theta) * np.sin(phi)
az = np.degrees(np.arctan2(x, z))      # 0 at +Z, +90 at +X
el = np.degrees(np.arcsin(np.clip(y, -1, 1)))
cam = 35.0                             # the viewer's starting azimuth
rel = (az - cam + 180) % 360 - 180     # relative to the camera: 0 toward it, −90 its left
def box(a, e, ha, he, k, soft=4.0):
    da = np.abs((rel - a + 180) % 360 - 180)
    fa = np.clip((ha - da) / soft + 0.5, 0, 1)
    fe = np.clip((he - np.abs(el - e)) / soft + 0.5, 0, 1)
    return fa * fe * k
sm = lambda e0, e1, t: np.clip((t - e0) / (e1 - e0), 0, 1) ** 2 * (3 - 2 * np.clip((t - e0) / (e1 - e0), 0, 1))
L = np.where(el < 0, 0.18, 0.04 + 0.35 * sm(40, 85, el))          # dark studio, a grey ceiling, a dim floor
L = L + box(-70, 35, 18, 16, 14)       # key: high on the camera's left, raking across the sides
L = L + box(60, 20, 16, 12, 1.2)       # fill: faint, camera right
L = L + box(0, 65, 30, 9, 2.5)         # overhead toward the camera
L = L + box(-125, 22, 4, 28, 9)        # rim strip, behind left
L = L + box(125, 22, 4, 28, 7)         # rim strip, behind right
img = np.stack([L, L, L * 0.98], -1).astype(np.float32)
# Radiance RGBE
m = img.max(-1)
e = np.ceil(np.log2(np.maximum(m, 1e-32)))
scale = np.where(m > 1e-32, 256.0 / np.exp2(e), 0)
rgbe = np.zeros((H, W, 4), np.uint8)
rgbe[..., :3] = np.clip(img * scale[..., None], 0, 255).astype(np.uint8)
rgbe[..., 3] = np.where(m > 1e-32, e + 128, 0).astype(np.uint8)
with open(sys.argv[1], "wb") as f:
    f.write(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n" + f"-Y {H} +X {W}\n".encode())
    f.write(rgbe.tobytes())
print("wrote", sys.argv[1])
