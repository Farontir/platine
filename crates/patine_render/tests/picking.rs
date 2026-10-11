//! M2 acceptance criterion 3: the picking buffer returns the right triangle on the test
//! meshes, compared with the CPU reference (`Bvh::closest_hit` along each pixel's ray).

use glam::{Vec2, Vec3};
use patine_core::mesh::MeshAsset;
use patine_gpu::{GpuContext, gpu_test, wgpu};
use patine_mesh::bvh::{Bvh, Ray};
use patine_mesh::primitives;
use patine_render::camera::OrbitCamera;
use patine_render::mesh_gpu::GpuMesh;
use patine_render::settings::ViewSettings;
use patine_render::viewport::{Scene, Viewport, ViewportOptions};

const WIDTH: u32 = 80;
const HEIGHT: u32 = 60;

/// Distance along `ray` to `triangle`, two-sided, if hit.
fn hit_distance(mesh: &MeshAsset, triangle: u32, ray: &Ray) -> Option<f32> {
    let p = mesh.triangles[triangle as usize].map(|v| Vec3::from(mesh.positions[v as usize]));
    let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
    let q = ray.direction.cross(e2);
    let det = e1.dot(q);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = ray.origin - p[0];
    let u = s.dot(q) / det;
    let r = s.cross(e1);
    let v = ray.direction.dot(r) / det;
    // Enlarged: rasterizers snap vertices to a sub-pixel grid (often 1/256 pixel), so a
    // pixel center within a small fraction of a pixel of an edge may go to either side.
    // 2e-3 is about a tenth of a pixel in these tests.
    let margin = 2e-3;
    (u >= -margin && v >= -margin && u + v <= 1.0 + margin).then(|| e2.dot(r) / det)
}

fn check_picking(ctx: &GpuContext, mesh: &MeshAsset, flat: bool) {
    let options = ViewportOptions {
        force_flat_picking: flat,
        ..ViewportOptions::small()
    };
    let mut viewport = Viewport::with_options(ctx, options).expect("viewport");
    // The viewport may fall back to flat picking on its own (driver without a usable
    // primitive_index); meshes then need the fallback buffers.
    let uses_flat = flat || viewport.needs_flat_pick_buffers();
    let gpu_mesh = if uses_flat {
        GpuMesh::with_flat_picking(ctx, mesh)
    } else {
        GpuMesh::new(ctx, mesh)
    }
    .expect("mesh upload");
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let mut camera = OrbitCamera {
        yaw: 0.7,
        pitch: 0.45,
        ..OrbitCamera::default()
    };
    camera.frame(&gpu_mesh.bounds, aspect);
    camera.zoom(0.8);
    let settings = ViewSettings::default();
    let scene = Scene {
        mesh: Some(&gpu_mesh),
        materials: None,
        camera: &camera,
        settings: &settings,
    };
    let image = viewport
        .pick_image(ctx, &scene, WIDTH, HEIGHT)
        .expect("picking pass")
        .expect("a mesh is shown");
    let bvh = Bvh::build(&mesh.positions, &mesh.triangles);
    let inverse = camera.view_projection(aspect).inverse();
    let scale = gpu_mesh.bounds.diagonal();
    let id_at = |x: i64, y: i64| {
        let (x, y) = (
            x.clamp(0, i64::from(WIDTH) - 1),
            y.clamp(0, i64::from(HEIGHT) - 1),
        );
        image.ids[(y * i64::from(WIDTH) + x) as usize]
    };

    let (mut exact, mut equivalent, mut boundary, mut hits) = (0, 0, 0, 0);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let index = (y * WIDTH + x) as usize;
            let ndc = Vec2::new(
                (x as f32 + 0.5) / WIDTH as f32 * 2.0 - 1.0,
                1.0 - (y as f32 + 0.5) / HEIGHT as f32 * 2.0,
            );
            let ray = camera.ray(ndc, aspect);
            let cpu = bvh.closest_hit(&mesh.positions, &mesh.triangles, &ray, f32::INFINITY);
            let gpu = image.ids[index].checked_sub(1);
            if gpu.is_some() && cpu.is_some() {
                hits += 1;
            }
            match (gpu, cpu) {
                (None, None) => continue,
                (Some(g), Some(c)) if g == c.triangle => {
                    exact += 1;
                    // The depth gives back the surface point. Skipped at grazing angles,
                    // where vertex snapping moves the point along the ray by 1 / cos.
                    let p =
                        mesh.triangles[g as usize].map(|v| Vec3::from(mesh.positions[v as usize]));
                    let normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
                    if normal.dot(ray.direction).abs() > 0.2 {
                        let position = inverse.project_point3(ndc.extend(image.depths[index]));
                        let expected = ray.origin + ray.direction * c.t;
                        assert!(
                            position.distance(expected) < 1e-3 * scale,
                            "pixel ({x}, {y}): depth position {position} vs {expected}"
                        );
                    }
                    continue;
                }
                // Pixel on a shared edge: the GPU triangle is hit at the same distance.
                (Some(g), Some(c))
                    if hit_distance(mesh, g, &ray)
                        .is_some_and(|t| (t - c.t).abs() < 1e-3 * scale) =>
                {
                    equivalent += 1;
                    continue;
                }
                _ => {}
            }
            // Remaining mismatches must sit on a boundary of the ID image (silhouette or
            // between triangles, where rasterization and ray precision differ).
            let (xi, yi) = (i64::from(x), i64::from(y));
            let own = image.ids[index];
            let on_boundary = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(dx, dy)| id_at(xi + dx, yi + dy) != own);
            assert!(
                on_boundary,
                "{} pixel ({x}, {y}): GPU {gpu:?} vs CPU {:?} inside a uniform region",
                mesh.name,
                cpu.map(|c| c.triangle)
            );
            boundary += 1;
        }
    }
    eprintln!(
        "{} (flat {uses_flat}): {hits} hits, {exact} exact, {equivalent} on edges, {boundary} on boundaries",
        mesh.name
    );
    assert!(
        hits > 500,
        "{}: the mesh covers too little of the image",
        mesh.name
    );
    assert!(
        exact * 100 >= hits * 97,
        "{}: {exact} exact of {hits}",
        mesh.name
    );
    assert!(
        boundary * 100 <= WIDTH * HEIGHT,
        "{}: {boundary} boundary mismatches",
        mesh.name
    );
}

gpu_test!(
    fn picking_returns_the_visible_triangle(ctx) {
        for mesh in [primitives::cube(), primitives::uv_sphere(32, 16), primitives::two_cubes()] {
            check_picking(&ctx, &mesh, false);
        }
    }
);

gpu_test!(
    fn picking_fallback_without_primitive_index(ctx) {
        for mesh in [primitives::cube(), primitives::uv_sphere(32, 16), primitives::two_cubes()] {
            check_picking(&ctx, &mesh, true);
        }
    }
);

gpu_test!(
    fn asynchronous_pick_matches_the_image(ctx) {
        let mesh = primitives::uv_sphere(32, 16);
        let mut viewport = Viewport::with_options(&ctx, ViewportOptions::small()).unwrap();
        let gpu_mesh = if viewport.needs_flat_pick_buffers() {
            GpuMesh::with_flat_picking(&ctx, &mesh)
        } else {
            GpuMesh::new(&ctx, &mesh)
        }
        .unwrap();
        let mut camera = OrbitCamera::default();
        camera.frame(&gpu_mesh.bounds, WIDTH as f32 / HEIGHT as f32);
        let settings = ViewSettings::default();
        let scene = Scene {
            mesh: Some(&gpu_mesh),
            materials: None,
            camera: &camera,
            settings: &settings,
        };
        let image = viewport.pick_image(&ctx, &scene, WIDTH, HEIGHT).unwrap().unwrap();
        for pixel in [[WIDTH / 2, HEIGHT / 2], [2, 2], [WIDTH / 2 + 7, HEIGHT / 2 - 5]] {
            viewport.request_pick(&ctx, &scene, WIDTH, HEIGHT, pixel);
            let result = loop {
                ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                if let Some(result) = viewport.poll_pick(&ctx) {
                    break result;
                }
            };
            let expected = image.ids[(pixel[1] * WIDTH + pixel[0]) as usize].checked_sub(1);
            assert_eq!(result.pixel, pixel);
            assert_eq!(result.triangle, expected, "pixel {pixel:?}");
            assert_eq!(result.position.is_some(), expected.is_some());
        }
    }
);
