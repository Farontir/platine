"""Generates the mesh fixtures of patine_mesh's import tests with Blender.

Every scene is built by this script (no external asset), then exported to glTF (binary
and separate), OBJ (+ .mtl) and FBX in tests/fixtures/meshes/. The outputs are CC0.

Run from the repository root:
    blender -b --factory-startup --python tests/fixtures/generate_fixtures.py
"""

import math
import os

import bpy

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "meshes")

# Unit cube, Blender is Z-up; faces counter-clockwise seen from outside.
CUBE_VERTS = [(-1, -1, -1), (1, -1, -1), (1, 1, -1), (-1, 1, -1),
              (-1, -1, 1), (1, -1, 1), (1, 1, 1), (-1, 1, 1)]
CUBE_FACES = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4),
              (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
SQUARE = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def material(name):
    found = bpy.data.materials.get(name)
    return found if found else bpy.data.materials.new(name)


def make_object(name, verts, faces, uv=None, materials=(), face_materials=None,
                colors=None, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1)):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], faces)
    if uv is not None:
        layer = mesh.uv_layers.new(name="UVMap")
        for poly in mesh.polygons:
            for k, loop in enumerate(poly.loop_indices):
                co = mesh.vertices[mesh.loops[loop].vertex_index].co
                layer.data[loop].uv = uv(poly.index, k, co)
    for m in materials:
        mesh.materials.append(m)
    if face_materials:
        for poly in mesh.polygons:
            poly.material_index = face_materials[poly.index]
    if colors:
        attribute = mesh.color_attributes.new(name="Color", type="FLOAT_COLOR",
                                              domain="CORNER")
        for index, loop in enumerate(mesh.loops):
            attribute.data[index].color = colors[loop.vertex_index]
        mesh.color_attributes.active_color = attribute
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    obj.location = location
    obj.rotation_euler = rotation
    obj.scale = scale
    return obj


def per_face_square(_poly, k, _co):
    return SQUARE[k]


def export(stem):
    for obj in bpy.context.scene.objects:
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
    base = os.path.join(OUT, stem)
    for fmt, ext in (("GLB", ".glb"), ("GLTF_SEPARATE", ".gltf")):
        bpy.ops.export_scene.gltf(filepath=base + ext, export_format=fmt,
                                  use_selection=True, export_yup=True,
                                  export_vertex_color="ACTIVE")
    bpy.ops.wm.obj_export(filepath=base + ".obj", export_selected_objects=True,
                          export_materials=True, export_uv=True, export_normals=True,
                          export_colors=True, export_triangulated_mesh=False,
                          forward_axis="NEGATIVE_Z", up_axis="Y")
    bpy.ops.export_scene.fbx(filepath=base + ".fbx", use_selection=True,
                             object_types={"MESH"}, mesh_smooth_type="OFF",
                             colors_type="SRGB", axis_forward="-Z", axis_up="Y",
                             apply_unit_scale=True, use_mesh_modifiers=False,
                             add_leaf_bones=False)


def cube_materials():
    reset()
    make_object("Cube", CUBE_VERTS, CUBE_FACES, uv=per_face_square,
                materials=[material("Painted"), material("Metal")],
                face_materials=[1, 1, 0, 0, 0, 0])
    export("cube_materials")


def concave_ngon():
    # L-shaped concave hexagon starting at a vertex that cannot see the whole polygon
    # (a fan from it would leave the shape), plus a quad sharing its right edge.
    reset()
    verts = [(2, 1, 0), (1, 1, 0), (1, 2, 0), (0, 2, 0), (0, 0, 0), (2, 0, 0),
             (3, 0, 0), (3, 1, 0)]
    faces = [(0, 1, 2, 3, 4, 5), (5, 6, 7, 0)]
    make_object("Ngon", verts, faces, uv=lambda _p, _k, co: (co.x / 3.0, co.y / 2.0),
                materials=[material("Default")])
    export("concave_ngon")


def multi_object():
    reset()
    shared = material("Shared")
    make_object("Left", CUBE_VERTS, CUBE_FACES, uv=per_face_square, materials=[shared],
                location=(-3, 0, 0), scale=(-1, 1, 1))
    make_object("Right", CUBE_VERTS, CUBE_FACES, uv=per_face_square, materials=[shared],
                location=(3, 0, 0), rotation=(0, 0, math.radians(45)),
                scale=(0.5, 0.5, 0.5))
    export("multi_object")


def no_uv():
    reset()
    make_object("Bare", CUBE_VERTS, CUBE_FACES, materials=[material("Default")])
    export("no_uv")


def vertex_colors():
    # Colors with components 0 or 1 survive any sRGB / linear convention unchanged.
    reset()
    verts = [(-1, -1, 0), (1, -1, 0), (1, 1, 0), (-1, 1, 0)]
    colors = [(1, 0, 0, 1), (0, 1, 0, 1), (0, 0, 1, 1), (1, 1, 1, 1)]
    make_object("Colored", verts, [(0, 1, 2, 3)],
                uv=lambda _p, _k, co: ((co.x + 1) / 2, (co.y + 1) / 2),
                materials=[material("Default")], colors=colors)
    export("vertex_colors")


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    for scene in (cube_materials, concave_ngon, multi_object, no_uv, vertex_colors):
        scene()
    print("fixtures written to", OUT)
