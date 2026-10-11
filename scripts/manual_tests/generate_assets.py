"""Generates the local manual-test assets with Blender (nothing downloaded).

Run from the repository root, after the downloads of manual_tests/assets:
    blender -b --factory-startup --python scripts/manual_tests/generate_assets.py

Outputs in manual_tests/assets/generated/:
- vintage_suitcase.obj (+ .mtl): OBJ conversion of the CC0 Poly Haven suitcase (FBX).
- suzanne_1m.glb: Blender's Suzanne subdivided to more than one million triangles,
  for the large-mesh tests (import stays responsive, viewport budget).
- unsupported.stl: a file of an unsupported type, for the error message test.
"""

import os

import bpy

ROOT = os.getcwd()
ASSETS = os.path.join(ROOT, "manual_tests", "assets")
OUT = os.path.join(ASSETS, "generated")


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def select_all_meshes():
    for obj in bpy.context.scene.objects:
        obj.select_set(obj.type == "MESH")
        if obj.type == "MESH":
            bpy.context.view_layer.objects.active = obj


def suitcase_obj():
    reset()
    bpy.ops.import_scene.fbx(filepath=os.path.join(ASSETS, "vintage_suitcase_1k.fbx"))
    select_all_meshes()
    bpy.ops.wm.obj_export(
        filepath=os.path.join(OUT, "vintage_suitcase.obj"),
        export_selected_objects=True,
        export_materials=True,
        export_uv=True,
        export_normals=True,
        export_triangulated_mesh=False,
        forward_axis="NEGATIVE_Z",
        up_axis="Y",
    )


def suzanne_1m():
    reset()
    bpy.ops.mesh.primitive_monkey_add(size=2.0, calc_uvs=True)
    obj = bpy.context.active_object
    modifier = obj.modifiers.new("subdivision", "SUBSURF")
    modifier.levels = 5
    modifier.render_levels = 5
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    bpy.ops.object.shade_smooth()
    select_all_meshes()
    bpy.ops.export_scene.gltf(
        filepath=os.path.join(OUT, "suzanne_1m.glb"),
        export_format="GLB",
        use_selection=True,
        export_yup=True,
    )


def unsupported_file():
    with open(os.path.join(OUT, "unsupported.stl"), "w", encoding="ascii") as f:
        f.write("solid empty\nendsolid empty\n")


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    suitcase_obj()
    suzanne_1m()
    unsupported_file()
    print("manual test assets written to", OUT)
