import bpy
import sys
from pathlib import Path


def get_args():
    if "--" not in sys.argv:
        raise RuntimeError(
            "Usage:\n"
            "blender --background --python resize_vrm.py -- "
            "input.vrm output.vrm 256"
        )

    args = sys.argv[sys.argv.index("--") + 1:]

    if len(args) < 2:
        raise RuntimeError(
            "Butuh minimal input dan output.\n"
            "Contoh:\n"
            "blender --background --python resize_vrm.py -- "
            "nera.vrm nera_256.vrm 256"
        )

    input_path = Path(args[0]).expanduser().resolve()
    output_path = Path(args[1]).expanduser().resolve()

    max_size = int(args[2]) if len(args) >= 3 else 256

    if not input_path.exists():
        raise FileNotFoundError(f"Input tidak ditemukan: {input_path}")

    if input_path == output_path:
        raise RuntimeError("Jangan overwrite VRM asli. Gunakan nama output berbeda.")

    if max_size < 1:
        raise ValueError("Ukuran texture harus > 0")

    return input_path, output_path, max_size


def clear_scene():
    print("\n[1/4] Membersihkan scene Blender...")

    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)


def import_vrm(input_path):
    print(f"\n[2/4] Import VRM:")
    print(f"      {input_path}")

    try:
        result = bpy.ops.import_scene.vrm(
            filepath=str(input_path)
        )
    except AttributeError as e:
        raise RuntimeError(
            "\nVRM Add-on tidak ditemukan / tidak aktif.\n"
            "Install dan enable 'VRM format' di Blender terlebih dahulu."
        ) from e

    if result != {"FINISHED"}:
        raise RuntimeError(f"Import VRM gagal: {result}")


def resize_images(max_size):
    print(f"\n[3/4] Resize texture -> max {max_size}px")

    total = 0
    resized = 0
    skipped = 0

    for image in list(bpy.data.images):
        name = image.name.lower()

        if image.name in {"Render Result", "Viewer Node"}:
            continue

        width, height = image.size[:]

        if width <= 0 or height <= 0:
            continue

        total += 1

        # Jangan sentuh texture yang mempengaruhi shading.
        if (
            "normal" in name
            or "_nml" in name
            or "emissive" in name
            or "matcap" in name
            or "shader_" in name
            or "thumbnail" in name
        ):
            print(f"  KEEP   {image.name}: {width}x{height}")
            skipped += 1
            continue

        if width <= max_size and height <= max_size:
            print(f"  SKIP   {image.name}: {width}x{height}")
            skipped += 1
            continue

        scale = max_size / max(width, height)

        new_width = max(1, round(width * scale))
        new_height = max(1, round(height * scale))

        print(
            f"  RESIZE {image.name}: "
            f"{width}x{height} -> {new_width}x{new_height}"
        )

        image.scale(new_width, new_height)
        image.update()

        try:
            image.pack()
        except RuntimeError:
            pass

        resized += 1

    print()
    print(f"  Total texture : {total}")
    print(f"  Resized       : {resized}")
    print(f"  Keep/Skip     : {skipped}")


def export_vrm(output_path):
    print("\n[4/4] Export VRM:")
    print(f"      {output_path}")

    output_path.parent.mkdir(parents=True, exist_ok=True)

    if output_path.exists():
        output_path.unlink()

    result = bpy.ops.export_scene.vrm(
        filepath=str(output_path)
    )

    if result != {"FINISHED"}:
        raise RuntimeError(f"Export VRM gagal: {result}")

    size_mb = output_path.stat().st_size / 1024 / 1024

    print("\n========================================")
    print(" SELESAI")
    print("========================================")
    print(f"Output : {output_path}")
    print(f"Size   : {size_mb:.2f} MB")
    print("========================================")


def main():
    input_path, output_path, max_size = get_args()

    print("========================================")
    print(" NERA VRM TEXTURE OPTIMIZER")
    print("========================================")
    print(f"Input     : {input_path}")
    print(f"Output    : {output_path}")
    print(f"Max size  : {max_size}px")
    print("========================================")

    clear_scene()
    import_vrm(input_path)
    resize_images(max_size)
    export_vrm(output_path)


if __name__ == "__main__":
    main()