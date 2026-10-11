"""Captures the screenshots of the manual-test tutorial with patine_app's capture mode.

Run from the repository root after `cargo build --release -p patine_app -p patine_cli`
and the manual-test assets (see generate_assets.py):
    python scripts/manual_tests/capture_tutorial.py

Writes manual_tests/captures/<name>.png (1600 x 900 client area, French interface).
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
        command = [APP] + args + ["--language", "fr", "--window", "1600x900", "--capture", target]
        # The last --language wins: interface_en passes its own before.
        if "--language" in args:
            command = [APP] + args + ["--window", "1600x900", "--capture", target]
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
