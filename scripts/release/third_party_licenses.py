"""Writes the license notices of every third-party crate shipped in the release binaries.

Usage (from the repository root, after `cargo fetch`):
    python scripts/release/third_party_licenses.py --target x86_64-pc-windows-msvc --out THIRD-PARTY-LICENSES.txt

Follows the normal and build dependencies of patine_app and patine_cli for the target
(dev-dependencies are not shipped), and copies the license files each crate ships, at its
root or one folder down (fonts/, a vendored library...). A crate without such a file is
listed with its declared license and authors; the texts of the MIT and Apache-2.0
licenses close the file.
"""

import argparse
import json
import os
import subprocess
import sys

ROOTS = {"patine_app", "patine_cli"}
PREFIXES = ("license", "licence", "copying", "notice", "copyright", "unlicense", "ofl", "ufl")
# Folders of code, not of notices.
SKIPPED = {"src", "tests", "benches", "examples", "target", ".git", ".github"}
REPOSITORY = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def metadata(target):
    output = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    ).stdout
    return json.loads(output)


def shipped_packages(meta):
    """Package ids reachable from the binaries through normal and build dependencies."""
    nodes = {node["id"]: node for node in meta["resolve"]["nodes"]}
    names = {package["id"]: package["name"] for package in meta["packages"]}
    todo = [pid for pid in nodes if names[pid] in ROOTS]
    seen = set()
    while todo:
        pid = todo.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            kinds = {kind["kind"] for kind in dep["dep_kinds"]}
            if kinds & {None, "build"}:
                todo.append(dep["pkg"])
    return seen


def is_notice(name):
    lower = name.lower()
    return lower.startswith(PREFIXES) or "license" in lower or "licence" in lower


def license_files(directory):
    """License files at the root of `directory`, then one folder down."""
    found = []
    for name in sorted(os.listdir(directory)):
        path = os.path.join(directory, name)
        if os.path.isfile(path) and is_notice(name):
            found.append(path)
        elif os.path.isdir(path) and name not in SKIPPED:
            for inner in sorted(os.listdir(path)):
                inner_path = os.path.join(path, inner)
                if os.path.isfile(inner_path) and is_notice(inner):
                    found.append(inner_path)
    return found


def license_texts(out):
    """The MIT and Apache-2.0 texts, for the crates that ship no license file."""
    out.write("=" * 79 + "\nLicense texts\n" + "=" * 79 + "\n\n")
    out.write(
        "Crates under the MIT license that ship no license file are covered by the\n"
        "following text, with the copyright of the authors listed above for each crate.\n\n"
    )
    with open(os.path.join(REPOSITORY, "LICENSE-MIT"), encoding="utf-8") as text:
        lines = text.read().strip().splitlines()
    # Without the copyright line of Patine itself.
    body = "\n".join(line for line in lines[1:] if not line.startswith("Copyright"))
    out.write("MIT License\n\nCopyright (c) the authors of the crate\n" + body + "\n\n")
    with open(os.path.join(REPOSITORY, "LICENSE-APACHE"), encoding="utf-8") as text:
        out.write(text.read().strip() + "\n")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    meta = metadata(args.target)
    members = set(meta["workspace_members"])
    shipped = shipped_packages(meta)
    packages = sorted(
        (p for p in meta["packages"] if p["id"] in shipped and p["id"] not in members),
        key=lambda p: (p["name"], p["version"]),
    )
    without_files = []
    with open(args.out, "w", encoding="utf-8", newline="\n") as out:
        out.write("Patine: third-party software\n")
        out.write("=" * 28 + "\n\n")
        out.write(
            "The Patine binaries include the following crates. Each one is listed with its\n"
            "declared license and the license files it ships.\n\n"
        )
        for package in packages:
            directory = os.path.dirname(package["manifest_path"])
            header = f"{package['name']} {package['version']} ({package.get('license') or 'see files'})"
            out.write("-" * 79 + "\n" + header + "\n")
            if package.get("repository"):
                out.write(package["repository"] + "\n")
            if package.get("authors"):
                out.write("Authors: " + ", ".join(package["authors"]) + "\n")
            out.write("-" * 79 + "\n\n")
            files = license_files(directory)
            if not files:
                without_files.append(package["name"])
                out.write(
                    f"Licensed under {package.get('license')}; the crate ships no license file "
                    "(see the license texts at the end).\n\n"
                )
            for path in files:
                with open(path, encoding="utf-8", errors="replace") as text:
                    out.write(f"[{os.path.relpath(path, directory)}]\n")
                    out.write(text.read().strip() + "\n\n")
        license_texts(out)
    print(f"{len(packages)} crates written to {args.out}", file=sys.stderr)
    if without_files:
        print(f"{len(without_files)} without license file: {', '.join(without_files)}", file=sys.stderr)


if __name__ == "__main__":
    main()
