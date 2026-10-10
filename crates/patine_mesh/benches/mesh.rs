//! Mesh pipeline benchmarks on a ~2 M triangle sphere (the bake budget of PLAN.md uses a
//! 2 M triangle high poly).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use glam::Vec3;
use patine_mesh::bvh::{Bvh, Ray};
use patine_mesh::primitives;
use patine_mesh::topology::MeshTopology;

fn mesh_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("sphere_2m");
    group.sample_size(10);

    group.bench_function("generate_and_assemble", |b| {
        b.iter(|| black_box(primitives::uv_sphere(1024, 1024)))
    });

    let mesh = primitives::uv_sphere(1024, 1024);
    eprintln!(
        "{} triangles, {} vertices",
        mesh.triangle_count(),
        mesh.vertex_count()
    );

    group.bench_function("topology", |b| {
        b.iter(|| black_box(MeshTopology::new(&mesh)))
    });
    group.bench_function("bvh_build", |b| {
        b.iter(|| black_box(Bvh::build(&mesh.positions, &mesh.triangles)))
    });

    let bvh = Bvh::build(&mesh.positions, &mesh.triangles);
    // 10 000 rays from a sphere of radius 3 towards points near the origin.
    let rays: Vec<Ray> = (0..10_000)
        .map(|i| {
            let f = i as f32;
            let origin =
                Vec3::new((f * 0.37).sin(), (f * 0.11).cos(), (f * 0.23).sin()).normalize() * 3.0;
            let target = Vec3::new((f * 0.71).sin(), (f * 0.53).cos(), (f * 0.19).sin()) * 0.5;
            Ray {
                origin,
                direction: target - origin,
            }
        })
        .collect();
    group.bench_function("closest_hit_10k_rays", |b| {
        b.iter(|| {
            rays.iter()
                .filter_map(|ray| {
                    bvh.closest_hit(&mesh.positions, &mesh.triangles, ray, f32::INFINITY)
                })
                .count()
        })
    });
    group.finish();
}

criterion_group!(benches, mesh_pipeline);
criterion_main!(benches);
