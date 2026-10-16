"""Captures the screenshots of the manual-test tutorial with patine_app's capture mode.

Run from the repository root after `cargo build --release -p patine_app -p patine_cli`
and the manual-test assets (see generate_assets.py):
    python scripts/manual_tests/capture_tutorial.py

Writes manual_tests/captures/<name>.png (1600 x 900 client area unless WINDOW says
otherwise, French interface).
Also writes manual_tests/casque.patine (a project made from the flight helmet) and a
simulated crashed session in manual_tests/autosave_demo, for the M3 captures.
"""

import os
import shutil
import subprocess
import sys

EXE = ".exe" if os.name == "nt" else ""
APP = os.path.join("target", "release", "patine_app" + EXE)
CLI = os.path.join("target", "release", "patine_cli" + EXE)
OUT = os.path.join("manual_tests", "captures")
A = os.path.join("manual_tests", "assets")
G = os.path.join(A, "generated")
F = os.path.join("tests", "fixtures", "meshes")
ENV = os.path.join("assets", "environments")

HELMET = os.path.join(A, "FlightHelmet.gltf")
BOOMBOX = os.path.join(A, "BoomBox.glb")
SUITCASE = os.path.join(A, "vintage_suitcase_1k.fbx")
SUITCASE_OBJ = os.path.join(G, "vintage_suitcase.obj")
LANTERN = os.path.join(A, "Lantern_01_1k.fbx")
SUZANNE = os.path.join(G, "suzanne_1m.glb")
NOON_EXR = os.path.join(A, "qwantani_noon_puresky_2k.exr")
PHOTOSTUDIO = os.path.join(A, "brown_photostudio_02_2k.hdr")
STUDIO_HDR = os.path.join(ENV, "studio_small_08_1k.hdr")
KLOOFENDAL = os.path.join(ENV, "kloofendal_48d_partly_cloudy_puresky_1k.hdr")

LANTERN_VIEW = ["--orbit", "35,12", "--zoom", "0.85"]

PROJECT = os.path.join("manual_tests", "casque.patine")
AUTOSAVE_DEMO = os.path.join("manual_tests", "autosave_demo")
HISTORY_EDITS = [
    "--edit", "rotation=60",
    "--edit", "exposure=0.7",
    "--edit", "rename=0:Verres",
    "--edit", "rotation=140",
    "--edit", "undo=1",
]

# A copper fill, then a blue fill whose base color blend each mode capture sets.
MODE_BASE = [
    "--tab", "layers",
    "--edit", "fill=0.85,0.5,0.25,0.4",
    "--edit", "fill=0.2,0.45,0.85,0.4",
]
# Leather tint, light fill in Overlay, both grouped, then a blue fill in Multiply.
M5_STACK = [
    "--edit", "fill=0.55,0.3,0.15,0.6",
    "--edit", "fill=0.9,0.85,0.7,0.4",
    "--edit", "mode=overlay,0.7",
    "--edit", "group=",
    "--edit", "fill=0.2,0.5,0.9,0.2",
    "--edit", "mode=multiply,0.5",
]

# M6: a red fill on the leather of the helmet, then a mask built step by step.
M6_BASE = [HELMET, "--orbit", "30,10", "--texture-set", "1", "--edit", "fill=0.8,0.12,0.08,0.45"]
M6_PANELS = ["--tab", "layers", "--tab", "properties"]
M6_FILL = ["--edit", "mask=black", "--edit", "mask-fill=0.6,normal,1"]
M6_LEVELS = M6_FILL + ["--edit", "mask-levels=0,1,2.5,0,1"]

# M7: procedural materials on the leather of the helmet; thumbnails need a few frames.
M7_BASE = [HELMET, "--orbit", "30,10", "--texture-set", "1", "--capture-frames", "60"]
M7_PANELS = ["--tab", "library", "--tab", "properties"]
SUITCASE_VIEW = ["--orbit", "30,20", "--zoom", "0.8"]

# M8: a paint layer on the first suitcase (texture set 0), strokes in viewport pixels.
M8_BASE = [SUITCASE, "--capture-frames", "60"] + SUITCASE_VIEW
M8_PAINT = ["--edit", "paint-layer=", "--brush", "size=50", "--brush", "color=0.75,0.12,0.08"]
M8_STROKES = ["--stroke", "590,270;760,285;905,275", "--stroke", "640,390;780,340;880,310"]
M8_ERASE = ["--stroke", "eraser:745,200;765,430"]
M8_MASK = [
    "--edit", "fill=0.15,0.45,0.85,0.3",
    "--edit", "mask=black",
    "--edit", "mask-paint=",
    "--brush", "size=70",
    "--stroke", "590,270;760,285;905,275",
    "--stroke", "640,390;780,340;880,310",
]

# M9: strokes in viewport pixels on the suitcase; pressure is the third number of a point.
M9_PANELS = ["--tab", "layers", "--tab", "properties"]
M9_PRESET_STROKE = ["--stroke", "590,270;700,300;800,285;905,275"]
M9_HEIGHT = [
    "--edit", "paint-layer=",
    "--edit", "height-force=3",
    "--brush", "off=base-color",
    "--brush", "off=roughness",
    "--brush", "height=1",
    "--brush", "size=40",
    "--brush", "hardness=0.9",
    "--stroke", "620,300;900,300",
    "--stroke", "620,380;900,380",
]

# M10: the same hard stroke on the suitcase seen close up, at two resolutions.
M10_BASE = [SUITCASE, "--capture-frames", "60", "--orbit", "30,20", "--zoom", "0.25"]
M10_STROKE = [
    "--edit", "paint-layer=",
    "--brush", "size=60", "--brush", "hardness=1", "--brush", "flow=1",
    "--brush", "color=0.75,0.12,0.08",
    "--stroke", "760,170;860,300;1000,260;1100,380",
]

# name -> arguments (mesh first, if any)
CAPTURES = {
    "interface_fr": [],
    "interface_en": ["--language", "en"],
    "about": ["--dialog", "about"],
    "new_project_dialog": ["--dialog", "new-project", "--dialog-path", HELMET],
    "error_missing_uvs": [
        "--dialog", "new-project", "--dialog-path", os.path.join(F, "no_uv.fbx"),
        "--dialog-create", "--capture-frames", "60",
    ],
    "error_unsupported": [
        "--dialog", "new-project", "--dialog-path", os.path.join(G, "unsupported.stl"),
        "--dialog-create", "--capture-frames", "60",
    ],
    "helmet_texture_sets": [HELMET, "--tab", "texture-sets", "--orbit", "30,10"],
    "helmet_console": [HELMET, "--tab", "console", "--orbit", "30,10"],
    "boombox_glb": [BOOMBOX, "--orbit", "200,15", "--tab", "texture-sets"],
    "suitcase_fbx": [SUITCASE, "--orbit", "30,20", "--tab", "texture-sets"],
    "suitcase_obj": [SUITCASE_OBJ, "--orbit", "30,20", "--tab", "texture-sets"],
    "suzanne_1m_console": [SUZANNE, "--orbit", "20,10", "--tab", "console"],
    "fixture_multi_object": [os.path.join(F, "multi_object.glb"), "--orbit", "30,25", "--tab", "texture-sets"],
    "fixture_ngon_wireframe": [os.path.join(F, "concave_ngon.obj"), "--orbit", "0,70", "--wireframe"],
    "display_panel": [LANTERN, "--tab", "display"] + LANTERN_VIEW,
    "material_metal": [LANTERN, "--material", "0.95,0.72,0.35,1,0.25"] + LANTERN_VIEW,
    "material_plastic": [LANTERN, "--material", "0.7,0.08,0.06,0,0.45"] + LANTERN_VIEW,
    "material_rough_metal": [LANTERN, "--material", "0.9,0.9,0.9,1,0.85"] + LANTERN_VIEW,
    "env_studio_generated": [LANTERN, "--hdri", "studio", "--background", "env:0"] + LANTERN_VIEW,
    "env_studio_small": [LANTERN, "--hdri", STUDIO_HDR, "--background", "env:0"] + LANTERN_VIEW,
    "env_kloofendal": [LANTERN, "--hdri", KLOOFENDAL, "--background", "env:0"] + LANTERN_VIEW,
    "env_photostudio": [LANTERN, "--hdri", PHOTOSTUDIO, "--background", "env:0"] + LANTERN_VIEW,
    "env_noon_exr": [LANTERN, "--hdri", NOON_EXR, "--background", "env:0"] + LANTERN_VIEW,
    "rotation_0": [HELMET, "--hdri", KLOOFENDAL, "--rotation", "0", "--orbit", "40,25", "--background", "env:0"],
    "rotation_180": [HELMET, "--hdri", KLOOFENDAL, "--rotation", "180", "--orbit", "40,25", "--background", "env:0"],
    "exposure_minus2": [LANTERN, "--hdri", KLOOFENDAL, "--exposure", "-2"] + LANTERN_VIEW,
    "exposure_plus2": [LANTERN, "--hdri", KLOOFENDAL, "--exposure", "2"] + LANTERN_VIEW,
    "tonemap_agx": [SUZANNE, "--hdri", NOON_EXR, "--exposure", "1.5", "--tonemap", "agx", "--material", "0.9,0.25,0.1,0,0.4", "--orbit", "20,10"],
    "tonemap_aces": [SUZANNE, "--hdri", NOON_EXR, "--exposure", "1.5", "--tonemap", "aces", "--material", "0.9,0.25,0.1,0,0.4", "--orbit", "20,10"],
    "tonemap_neutral": [SUZANNE, "--hdri", NOON_EXR, "--exposure", "1.5", "--tonemap", "neutral", "--material", "0.9,0.25,0.1,0,0.4", "--orbit", "20,10"],
    "tonemap_linear": [SUZANNE, "--hdri", NOON_EXR, "--exposure", "1.5", "--tonemap", "linear", "--material", "0.9,0.25,0.1,0,0.4", "--orbit", "20,10"],
    "background_blur": [LANTERN, "--hdri", KLOOFENDAL, "--background", "env:0.7"] + LANTERN_VIEW,
    "background_solid": [LANTERN, "--hdri", KLOOFENDAL, "--background", "solid:0.05,0.05,0.06"] + LANTERN_VIEW,
    "shadows_on": [HELMET, "--hdri", KLOOFENDAL, "--rotation", "180", "--orbit", "40,25"],
    "shadows_off": [HELMET, "--hdri", KLOOFENDAL, "--rotation", "180", "--orbit", "40,25", "--no-shadows"],
    "wireframe": [SUITCASE, "--wireframe", "--orbit", "30,20", "--zoom", "0.8"],
    "projection_perspective": [SUITCASE, "--orbit", "45,30", "--fov", "70"],
    "projection_ortho": [SUITCASE, "--orbit", "45,30", "--ortho"],
    "picking_hover": [HELMET, "--orbit", "30,10", "--hover", "580,240", "--capture-frames", "45"],
    "m3_history": [HELMET, "--tab", "history", "--orbit", "30,10"] + HISTORY_EDITS,
    "m3_renamed": [HELMET, "--tab", "texture-sets", "--orbit", "30,10", "--edit", "rename=0:Verres"],
    "m3_opened": [PROJECT, "--tab", "history", "--orbit", "30,10"],
    "m3_recovery": [],
    "m4_texture_sets": [HELMET, "--tab", "texture-sets", "--orbit", "30,10", "--texture-set", "1"],
    "m4_channel_base": [HELMET, "--orbit", "30,10", "--display", "channel:base-color", "--material", "0.8,0.2,0.1,0,0.3"],
    "m4_channel_roughness": [LANTERN, "--display", "channel:roughness", "--material", "0.95,0.72,0.35,1,0.65"] + LANTERN_VIEW,
    "m4_checker": [HELMET, "--orbit", "30,10", "--display", "checker"],
    "m4_view_2d_helmet": [HELMET, "--tab", "view-2d", "--texture-set", "1"],
    "m4_view_2d_suitcase": [SUITCASE, "--tab", "view-2d"],
    "m5_empty": [HELMET, "--tab", "layers", "--tab", "properties", "--orbit", "30,10", "--texture-set", "1"],
    "m5_fill": [LANTERN, "--tab", "layers", "--tab", "properties", "--edit", "fill=0.75,0.12,0.08,0.35"] + LANTERN_VIEW,
    "m5_mode_normal": [LANTERN] + MODE_BASE + ["--edit", "mode=normal,0.5"] + LANTERN_VIEW,
    "m5_mode_multiply": [LANTERN] + MODE_BASE + ["--edit", "mode=multiply,1"] + LANTERN_VIEW,
    "m5_mode_screen": [LANTERN] + MODE_BASE + ["--edit", "mode=screen,1"] + LANTERN_VIEW,
    "m5_mode_overlay": [LANTERN] + MODE_BASE + ["--edit", "mode=overlay,1"] + LANTERN_VIEW,
    "m5_stack": [HELMET, "--tab", "layers", "--tab", "properties", "--orbit", "30,10", "--texture-set", "1"] + M5_STACK,
    "m5_stack_history": [HELMET, "--tab", "layers", "--tab", "history", "--orbit", "30,10", "--texture-set", "1"] + M5_STACK,
    "m6_black": M6_BASE + M6_PANELS + ["--edit", "mask=black"],
    "m6_white": M6_BASE + M6_PANELS + ["--edit", "mask=white"],
    "m6_fill": M6_BASE + M6_PANELS + M6_FILL,
    "m6_fill_mask": M6_BASE + M6_PANELS + M6_FILL + ["--display", "mask"],
    "m6_levels": M6_BASE + M6_PANELS + M6_LEVELS,
    "m6_levels_mask": M6_BASE + M6_PANELS + M6_LEVELS + ["--display", "mask"],
    "m6_invert_mask": M6_BASE + M6_PANELS + M6_LEVELS + ["--edit", "mask-invert=", "--display", "mask"],
    "m6_view_2d": [HELMET, "--tab", "view-2d", "--texture-set", "1", "--edit", "fill=0.8,0.12,0.08,0.45"] + M6_LEVELS + ["--display", "mask"],
    "m6_history": M6_BASE + ["--tab", "layers", "--tab", "history"] + M6_LEVELS + ["--edit", "mask-invert="],
    "m7_library": M7_BASE + M7_PANELS + ["--edit", "material=material.copper", "--edit", "param=oxidation:0.5", "--edit", "projection=triplanar,2"],
    "m7_steel": M7_BASE + ["--edit", "material=material.steel_brushed"],
    "m7_copper": M7_BASE + ["--edit", "material=material.copper", "--edit", "param=oxidation:0.5", "--edit", "projection=triplanar,2"],
    "m7_plastic": M7_BASE + ["--edit", "material=material.plastic_grained"],
    "m7_wood": M7_BASE + ["--edit", "material=material.wood", "--edit", "projection=triplanar,2"],
    "m7_concrete": M7_BASE + ["--edit", "material=material.concrete"],
    "m7_suitcase_uv": [SUITCASE, "--capture-frames", "60", "--edit", "material=material.wood", "--edit", "param=rings:24"] + SUITCASE_VIEW,
    "m7_suitcase_triplanar": [SUITCASE, "--capture-frames", "60", "--edit", "material=material.wood", "--edit", "param=rings:24", "--edit", "projection=triplanar,2"] + SUITCASE_VIEW,
    "m7_seed": M7_BASE + M7_PANELS + ["--edit", "material=material.concrete", "--edit", "param=seed:4242", "--edit", "param=stains:0.9"],
    "m7_mask": M7_BASE + ["--tab", "layers", "--tab", "properties", "--edit", "fill=0.8,0.1,0.08,0.4", "--edit", "mask=black", "--edit", "mask-procedural=noise.perlin", "--edit", "param=scale:6"],
    "m7_mask_view": M7_BASE + ["--tab", "layers", "--tab", "properties", "--edit", "fill=0.8,0.1,0.08,0.4", "--edit", "mask=black", "--edit", "mask-procedural=noise.perlin", "--edit", "param=scale:6", "--display", "mask"],
    "m7_history": M7_BASE + ["--tab", "layers", "--tab", "history", "--edit", "material=material.copper", "--edit", "param=oxidation:0.5", "--edit", "projection=triplanar,2"],
    "m8_stroke": M8_BASE + ["--tab", "layers", "--tab", "properties"] + M8_PAINT + M8_STROKES,
    "m8_eraser": M8_BASE + ["--tab", "layers", "--tab", "history"] + M8_PAINT + M8_STROKES + M8_ERASE,
    "m8_roughness": M8_BASE + M8_PAINT + M8_STROKES + ["--display", "channel:roughness"],
    "m8_alpha": M8_BASE + M8_PAINT + ["--brush", "size=130", "--brush", "alpha=alpha.splatter", "--brush", "spacing=0.6", "--stroke", "600,280;890,280"],
    "m8_cursor": M8_BASE + ["--tab", "layers", "--tab", "properties"] + M8_PAINT + ["--brush", "size=90", "--hover", "760,330"],
    "m8_mask": M8_BASE + ["--tab", "layers", "--tab", "properties"] + M8_MASK,
    "m8_mask_view": M8_BASE + ["--tab", "layers", "--tab", "properties"] + M8_MASK + ["--display", "mask"],
    "m9_pressure": M8_BASE + M8_PAINT + [
        "--brush", "size=70", "--brush", "hardness=1", "--brush", "flow=1",
        "--brush", "pressure-opacity=linear",
        "--stroke", "620,300,0.05;760,300,1;900,300,0.05",
        "--stroke", "620,420,0.1;900,420,1",
    ],
    "m9_panel": M8_BASE + M9_PANELS + M8_PAINT + [
        "--preset", "chalk", "--brush", "pressure-opacity=linear",
        "--symmetry", "x,radial=4", "--expand-brush",
    ],
    "m9_chalk": M8_BASE + M8_PAINT + ["--preset", "chalk"] + M9_PRESET_STROKE,
    "m9_splatter": M8_BASE + M8_PAINT + ["--preset", "splatter", "--brush", "size=70"] + M9_PRESET_STROKE,
    "m9_scratches": M8_BASE + M8_PAINT + ["--preset", "scratches", "--brush", "size=20"] + M9_PRESET_STROKE,
    "m9_airbrush": M8_BASE + M8_PAINT + ["--preset", "airbrush"] + M9_PRESET_STROKE,
    "m9_symmetry_helmet": [
        HELMET, "--orbit", "0,10", "--texture-set", "1", "--capture-frames", "60",
        "--edit", "paint-layer=", "--brush", "size=16", "--brush", "color=0.8,0.1,0.08",
        "--symmetry", "x",
        "--stroke", "552,82;540,110;532,140", "--stroke", "575,70;555,95",
    ],
    "m9_symmetry_lantern": [
        LANTERN, "--orbit", "35,30", "--zoom", "0.85", "--capture-frames", "60",
        "--edit", "paint-layer=", "--brush", "size=22", "--brush", "color=0.1,0.35,0.85",
        "--symmetry", "radial=6", "--stroke", "535,505;538,545",
    ],
    "m9_line": M8_BASE + M8_PAINT + ["--stroke", "600,250;700,262", "--stroke", "line:880,420"],
    "m9_uv": M8_BASE + ["--tab", "view-2d"] + M8_PAINT + [
        "--brush", "size=40", "--stroke", "uv:300,300;500,350;700,320",
    ],
    "m9_material": M8_BASE + M8_PAINT + [
        "--brush", "material=material.copper", "--brush", "size=120",
        "--stroke", "620,300;760,330;900,300",
    ],
    "m9_height": M8_BASE + ["--tab", "texture-sets"] + M9_HEIGHT,
    "m10_1k": M10_BASE + ["--edit", "resolution=1024"] + M10_STROKE,
    "m10_4k": M10_BASE + ["--tab", "texture-sets", "--edit", "resolution=4096"] + M10_STROKE,
}

# Captures in a window other than 1600 x 900.
WINDOW = {
    "m9_panel": "1600x1600",
}

# Captures run with extra environment variables.
ENVIRONMENT = {
    "m3_recovery": {"PATINE_AUTOSAVE_DIR": AUTOSAVE_DEMO},
}


def prepare_m3():
    """The project file and the crashed session the M3 captures need."""
    subprocess.run(
        [CLI, "bench", "project", HELMET, "--out", PROJECT],
        check=True, capture_output=True, timeout=300,
    )
    # A session folder whose lock nobody holds: what a crash leaves behind.
    shutil.rmtree(AUTOSAVE_DEMO, ignore_errors=True)
    session = os.path.join(AUTOSAVE_DEMO, "1760000000000-4242")
    os.makedirs(session)
    shutil.copy(PROJECT, os.path.join(session, "project.patine"))
    with open(os.path.join(session, "info.ron"), "w", encoding="utf-8") as info:
        info.write('(name: "casque", original_path: Some("manual_tests/casque.patine"))')
    open(os.path.join(session, "session.lock"), "w").close()


def main():
    os.makedirs(OUT, exist_ok=True)
    prepare_m3()
    only = set(sys.argv[1:])
    failures = []
    for name, args in CAPTURES.items():
        if only and name not in only:
            continue
        target = os.path.join(OUT, f"{name}.png")
        window = WINDOW.get(name, "1600x900")
        command = [APP] + args + ["--language", "fr", "--window", window, "--capture", target]
        # The last --language wins: interface_en passes its own before.
        if "--language" in args:
            command = [APP] + args + ["--window", window, "--capture", target]
        env = dict(os.environ, **ENVIRONMENT.get(name, {}))
        result = subprocess.run(command, capture_output=True, text=True, timeout=300, env=env)
        ok = result.returncode == 0 and os.path.exists(target)
        print(("ok    " if ok else "FAIL  ") + name)
        if not ok:
            failures.append(name)
            print(result.stdout[-2000:], result.stderr[-2000:])
    if failures:
        sys.exit(f"failed captures: {', '.join(failures)}")


if __name__ == "__main__":
    main()
