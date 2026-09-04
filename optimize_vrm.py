import bpy

MODULE = "bl_ext.blender_org.vrm"

try:
    bpy.ops.preferences.addon_enable(module=MODULE)
except Exception as e:
    print("ENABLE ERROR:", e)

print("VRM AVAILABLE:", hasattr(bpy.ops.import_scene, "vrm"))

INPUT = r"C:\Users\sahid\Documents\ai\assets\vrm\nera.vrm"
# Ubah nama output agar file asli 1k tidak tertimpa dulu
OUTPUT = r"C:\Users\sahid\Documents\ai\assets\vrm\nera_1k_optimized.vrm"

MAX_SIZE = 1024

bpy.ops.import_scene.vrm(filepath=INPUT)

print("=== RESIZING TEXTURES ===")
for image in bpy.data.images:
    w, h = image.size

    if w <= 0 or h <= 0:
        continue

    if w <= MAX_SIZE and h <= MAX_SIZE:
        print(f"SKIP {image.name}: {w}x{h}")
        continue

    scale = min(MAX_SIZE / w, MAX_SIZE / h)

    new_w = max(1, int(w * scale))
    new_h = max(1, int(h * scale))

    print(f"RESIZE {image.name}: {w}x{h} -> {new_w}x{new_h}")
    image.scale(new_w, new_h)

print("=== CLEANING UNUSED MORPH TARGETS (SHAPE KEYS) ===")
# Daftar Morph Target dasar yang dibiarkan hidup.
# Hapus "Angry"/"Sorrow" jika Nera di desktop pet-mu tidak pernah marah/sedih.
KEEP_SHAPES = ["Blink", "A", "I", "U", "E", "O", "Joy", "Angry", "Sorrow", "Fun", "Neutral"]

deleted_count = 0
for obj in bpy.data.objects:
    if obj.type == 'MESH' and obj.data.shape_keys:
        shape_keys = obj.data.shape_keys.key_blocks
        # Loop terbalik (reverse) agar index tidak bergeser saat kita menghapus data
        for i in range(len(shape_keys)-1, 0, -1):
            sk = shape_keys[i]
            # Jika namanya TIDAK ada di daftar KEEP_SHAPES, musnahkan!
            if not any(keep.lower() in sk.name.lower() for keep in KEEP_SHAPES):
                print(f"DELETING Shape Key: {sk.name} on {obj.name}")
                obj.shape_key_remove(sk)
                deleted_count += 1

print(f"TOTAL SHAPE KEYS DELETED: {deleted_count}")

print("=== EXPORTING ===")
bpy.ops.export_scene.vrm(filepath=OUTPUT)

print("DONE:", OUTPUT)