//! Headless Patine command line: bake and export projects without any UI dependency.
//!
//! Commands:
//! - `patine_cli inspect <mesh> [--allow-missing-uvs]`: import a mesh and print its
//!   statistics and the duration of each step.
//! - `patine_cli bench viewport [--triangles N] [--frames N] [--size WxH] [--no-shadows]`:
//!   frames per second of the viewport on the hardware adapter.
//! - `patine_cli bench project <mesh> [--out file.patine]`: creates a project from a mesh,
//!   saves it, opens it again, checks that it is identical, and prints the durations and
//!   the file size. `--out` keeps the file.
//! - `patine_cli --version`

mod bench;

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use patine_core::file;
use patine_core::project::{Project, ProjectSettings};
use patine_mesh::bvh::Bvh;
use patine_mesh::topology::MeshTopology;
use patine_mesh::{BuildOptions, import};

const USAGE: &str = "usage: patine_cli inspect <mesh> [--allow-missing-uvs]
       patine_cli bench viewport [--triangles N] [--frames N] [--size WxH] [--no-shadows]
       patine_cli bench project <mesh> [--out file.patine]
       patine_cli --version";

fn ms(duration: Duration) -> String {
    format!("{:.1} ms", duration.as_secs_f64() * 1e3)
}

fn inspect(path: &Path, require_uvs: bool) -> anyhow::Result<()> {
    let start = Instant::now();
    let imported = import(path, BuildOptions { require_uvs })
        .with_context(|| format!("cannot import {}", path.display()))?;
    let import_time = start.elapsed();
    let mesh = &imported.mesh;
    let report = &imported.report;

    let start = Instant::now();
    let topology = MeshTopology::new(mesh);
    let topology_time = start.elapsed();
    let start = Instant::now();
    let bvh = Bvh::build(&mesh.positions, &mesh.triangles);
    let bvh_time = start.elapsed();

    println!("{} ({})", path.display(), report.format.name());
    println!("  triangles  {}", mesh.triangle_count());
    println!("  vertices   {}", mesh.vertex_count());
    println!("  objects    {}", mesh.objects.len());
    println!("  parts      {}", topology.part_count());
    match topology.island_count() {
        Some(islands) => println!("  uv islands {islands}"),
        None => println!("  uv islands none (no UVs)"),
    }
    println!(
        "  colors     {}",
        if mesh.colors.is_some() { "yes" } else { "no" }
    );
    if let Some(bounds) = mesh.bounds() {
        println!("  bounds     {:?} .. {:?}", bounds.min, bounds.max);
    }
    println!("  materials");
    for (index, name) in mesh.materials.iter().enumerate() {
        println!(
            "    {name}: {} triangles",
            mesh.material_triangle_count(index as u32)
        );
    }

    let build = &report.build;
    println!("  report");
    println!("    source triangles      {}", build.source_triangles);
    println!("    degenerate removed    {}", build.degenerate_removed);
    println!(
        "    invalid normals fixed {}",
        build.invalid_normals_replaced
    );
    println!(
        "    uvs outside 0..1      {}",
        build.uvs_outside_unit_square
    );
    if !build.unused_materials.is_empty() {
        println!(
            "    unused materials      {}",
            build.unused_materials.join(", ")
        );
    }
    for warning in &report.warnings {
        println!("    warning: {warning}");
    }
    println!("  timings");
    println!("    read      {}", ms(report.read_time));
    for (step, duration) in &build.timings {
        println!("    {step:<9} {}", ms(*duration));
    }
    println!("    import    {} (total)", ms(import_time));
    println!("    topology  {}", ms(topology_time));
    println!(
        "    bvh       {} ({} nodes, depth {})",
        ms(bvh_time),
        bvh.nodes.len(),
        bvh.depth()
    );
    Ok(())
}

/// Saves and opens a project made from `mesh`, and prints the durations.
fn bench_project(mesh: &Path, out: Option<&Path>) -> anyhow::Result<()> {
    let start = Instant::now();
    let imported = import(mesh, BuildOptions::default())
        .with_context(|| format!("cannot import {}", mesh.display()))?;
    let import_time = start.elapsed();
    let project = Project::new(
        imported.mesh,
        mesh.to_path_buf(),
        ProjectSettings::default(),
    );

    let path = match out {
        Some(path) => file::with_extension(path),
        None => std::env::temp_dir().join(format!("patine-bench-{}.patine", std::process::id())),
    };
    let start = Instant::now();
    file::save(&project, &path, None).with_context(|| format!("cannot save {}", path.display()))?;
    let save_time = start.elapsed();
    let size = std::fs::metadata(&path)?.len();
    let start = Instant::now();
    let read = file::load(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let open_time = start.elapsed();
    if out.is_none() {
        std::fs::remove_file(&path)?;
    }
    if read != project {
        bail!("the project read back differs from the one saved");
    }

    let mesh_bytes = {
        let m = &project.mesh;
        m.positions.len() * 12
            + m.normals.len() * 12
            + m.uv0.as_ref().map_or(0, |uv| uv.len() * 8)
            + m.tangents.as_ref().map_or(0, |t| t.len() * 16)
            + m.colors.as_ref().map_or(0, |c| c.len() * 16)
            + m.triangles.len() * 20
    };
    println!("{}", mesh.display());
    println!("  triangles   {}", project.mesh.triangle_count());
    println!("  import      {}", ms(import_time));
    println!("  save        {}", ms(save_time));
    println!("  open        {}", ms(open_time));
    println!(
        "  file        {:.1} MB (mesh data {:.1} MB uncompressed)",
        size as f64 / 1e6,
        mesh_bytes as f64 / 1e6
    );
    println!("  round trip  identical");
    if out.is_some() {
        println!("  written     {}", path.display());
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("patine_cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("bench") if args.get(1).map(String::as_str) == Some("viewport") => {
            bench::ViewportBench::parse(&args[2..])?.run()
        }
        Some("bench") if args.get(1).map(String::as_str) == Some("project") => {
            let Some(mesh) = args.get(2) else {
                bail!("missing mesh path\n{USAGE}");
            };
            let out = match args.get(3).map(String::as_str) {
                Some("--out") => match args.get(4) {
                    Some(path) => Some(Path::new(path)),
                    None => bail!("--out needs a path\n{USAGE}"),
                },
                Some(other) => bail!("unknown option {other}\n{USAGE}"),
                None => None,
            };
            bench_project(Path::new(mesh), out)
        }
        Some("inspect") => {
            let Some(path) = args.get(1) else {
                bail!("missing mesh path\n{USAGE}");
            };
            let allow_missing_uvs = args[2..].iter().any(|a| a == "--allow-missing-uvs");
            inspect(Path::new(path), !allow_missing_uvs)
        }
        _ => bail!("{USAGE}"),
    }
}
