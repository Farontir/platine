"""Captures the screenshots of the manual-test tutorial with patine_app's capture mode.

Run from the repository root after `cargo build --release -p patine_app` and the
manual-test assets (see generate_assets.py):
    python scripts/manual_tests/capture_tutorial.py

Writes manual_tests/captures/<name>.png (1600 x 900 client area, French interface).
"""

import os
import subprocess
import sys

APP = os.path.join("target", "release", "patine_app.exe" if os.name == "nt" else "patine_app")
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
    "boombox_glb": [BOOMBOX, "--orbit", "25,15", "--tab", "texture-sets"],
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
    "rotation_0": [HELMET, "--hdri", NOON_EXR, "--rotation", "0", "--orbit", "30,25"],
    "rotation_150": [HELMET, "--hdri", NOON_EXR, "--rotation", "150", "--orbit", "30,25"],
    "exposure_minus2": [LANTERN, "--hdri", KLOOFENDAL, "--exposure", "-2"] + LANTERN_VIEW,
    "exposure_plus2": [LANTERN, "--hdri", KLOOFENDAL, "--exposure", "2"] + LANTERN_VIEW,
    "tonemap_agx": [SUZANNE, "--hdri", NOON_EXR, "--exposure", "1.5", "--tonemap", "agx", "--material", "0.9,0.25,0.1,0,0.4", "--orbit", "20,10"],
    "tonemap_aces": [SUZANNE, "--hdri", NOON_EXR, "--exposure", "1.5", "--tonemap", "aces", "--material", "0.9,0.25,0.1,0,0.4", "--orbit", "20,10"],
    "tonemap_neutral": [SUZANNE, "--hdri", NOON_EXR, "--exposure", "1.5", "--tonemap", "neutral", "--material", "0.9,0.25,0.1,0,0.4", "--orbit", "20,10"],
    "tonemap_linear": [SUZANNE, "--hdri", NOON_EXR, "--exposure", "1.5", "--tonemap", "linear", "--material", "0.9,0.25,0.1,0,0.4", "--orbit", "20,10"],
    "background_blur": [LANTERN, "--hdri", KLOOFENDAL, "--background", "env:0.7"] + LANTERN_VIEW,
    "background_solid": [LANTERN, "--hdri", KLOOFENDAL, "--background", "solid:0.05,0.05,0.06"] + LANTERN_VIEW,
    "shadows_on": [HELMET, "--hdri", NOON_EXR, "--rotation", "60", "--orbit", "40,30"],
    "shadows_off": [HELMET, "--hdri", NOON_EXR, "--rotation", "60", "--orbit", "40,30", "--no-shadows"],
    "wireframe": [SUITCASE, "--wireframe", "--orbit", "30,20", "--zoom", "0.8"],
    "projection_perspective": [SUITCASE, "--orbit", "45,30", "--fov", "70"],
    "projection_ortho": [SUITCASE, "--orbit", "45,30", "--ortho"],
    "picking_hover": [HELMET, "--orbit", "30,10", "--hover", "580,240", "--capture-frames", "45"],
}


def main():
    os.makedirs(OUT, exist_ok=True)
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
        result = subprocess.run(command, capture_output=True, text=True, timeout=300)
        ok = result.returncode == 0 and os.path.exists(target)
        print(("ok    " if ok else "FAIL  ") + name)
        if not ok:
            failures.append(name)
            print(result.stdout[-2000:], result.stderr[-2000:])
    if failures:
        sys.exit(f"failed captures: {', '.join(failures)}")


if __name__ == "__main__":
    main()
