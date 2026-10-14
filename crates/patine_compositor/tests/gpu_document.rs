//! M4 acceptance criterion 1: changing the resolution reallocates without leaking, and
//! the measured memory returns to the expected value. The document changes through real
//! commands and undo, as in the application.

use patine_compositor::compositor::Compositor;
use patine_compositor::gpu_document::{GpuDocument, expected_memory};
use patine_core::channel::{Channel, ChannelDef};
use patine_core::commands::{SetChannelDef, SetTextureSetResolution};
use patine_core::history::History;
use patine_core::mesh::{MeshAsset, Submesh};
use patine_core::project::{Project, ProjectSettings, Resolution, TextureSetId};
use patine_gpu::pool::{TexturePool, allocated_bytes};
use patine_gpu::{GpuContext, gpu_test, readback, wgpu};

/// Two triangles, two materials: two texture sets.
fn project() -> Project {
    let mesh = MeshAsset {
        name: "quad".into(),
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        uv0: Some(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]),
        tangents: Some(vec![[1.0, 0.0, 0.0, 1.0]; 4]),
        colors: None,
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        triangle_polygon: vec![0, 1],
        triangle_object: vec![0, 0],
        objects: vec!["quad".into()],
        materials: vec!["a".into(), "b".into()],
        submeshes: vec![
            Submesh {
                material: 0,
                triangles: 0..1,
            },
            Submesh {
                material: 1,
                triangles: 1..2,
            },
        ],
    };
    let settings = ProjectSettings {
        resolution: Resolution::new(512).expect("supported"),
        ..ProjectSettings::default()
    };
    Project::new(mesh, "quad.glb".into(), settings)
}

/// GPU state after a sync, checked against the project.
struct Fixture {
    pool: TexturePool,
    document: GpuDocument,
    compositor: Compositor,
}

impl Fixture {
    fn sync(&mut self, ctx: &GpuContext, project: &Project) -> Vec<TextureSetId> {
        let changed = self
            .document
            .sync(ctx, &mut self.pool, &self.compositor, project);
        // Destructions are processed by the device at its next maintenance.
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        assert_eq!(
            self.pool.memory(),
            expected_memory(project),
            "pool accounting"
        );
        assert_eq!(self.document.memory(), expected_memory(project));
        changed
    }
}

gpu_test!(
    fn resolution_changes_reallocate_without_leaking(ctx) {
        let mut project = project();
        let mut history = History::default();
        let mut gpu = Fixture {
            pool: TexturePool::new(u64::MAX),
            document: GpuDocument::new(),
            compositor: Compositor::new(&ctx).expect("shaders compile"),
        };
        let first = gpu.sync(&ctx, &project);
        assert_eq!(first, [TextureSetId(0), TextureSetId(1)]);
        let initial = gpu.pool.memory();
        let device_initial = allocated_bytes(&ctx);
        assert!(gpu.sync(&ctx, &project).is_empty(), "nothing to do twice");

        // 512 → 2048 → back by undo, on the first texture set only.
        let big = Resolution::new(2048).expect("supported");
        let command = SetTextureSetResolution::new(TextureSetId(0), big);
        history.push(&mut project, Box::new(command)).unwrap();
        assert_eq!(gpu.sync(&ctx, &project), [TextureSetId(0)]);
        // One set of two at 16 times the surface (the smallest mips scale less; the
        // exact value is checked against expected_memory by sync).
        let memory = gpu.pool.memory();
        assert!(memory > 8 * initial && memory < 9 * initial, "{memory}");
        history.undo(&mut project).unwrap();
        assert_eq!(gpu.sync(&ctx, &project), [TextureSetId(0)]);
        assert_eq!(gpu.pool.memory(), initial, "memory returns to its value");

        // Channels: add one, remove one, undo both.
        history.seal();
        let add = SetChannelDef::set(TextureSetId(1), ChannelDef::standard(Channel::Emissive));
        history.push(&mut project, Box::new(add)).unwrap();
        history.seal();
        let remove = SetChannelDef::remove(TextureSetId(1), Channel::Metallic);
        history.push(&mut project, Box::new(remove)).unwrap();
        assert_eq!(gpu.sync(&ctx, &project), [TextureSetId(1)]);
        history.go_to(&mut project, 0).unwrap();
        gpu.sync(&ctx, &project);
        assert_eq!(gpu.pool.memory(), initial);
        assert_eq!(
            gpu.pool.len(),
            12,
            "two sets of five channels, each with its shading normal"
        );

        match (device_initial, allocated_bytes(&ctx)) {
            (Some(before), Some(after)) => {
                // The device also counts wgpu's own transient allocations (command
                // buffers, staging), which come and go by a few hundred bytes on Vulkan.
                // A single leaked channel texture (the smallest, 512² R16Float with mips,
                // is about 700 KB) would be far above this margin.
                let margin = 64 * 1024;
                assert!(
                    after.abs_diff(before) < margin,
                    "device memory before {before}, after {after}"
                );
            }
            _ => eprintln!("the backend does not report allocations: pool accounting only"),
        }

        gpu.document.clear(&mut gpu.pool);
        assert_eq!(gpu.pool.memory(), 0);
    }
);

gpu_test!(
    /// Every mip level holds the channel's default value, and follows its changes.
    fn textures_hold_the_default_values(ctx) {
        let mut project = project();
        let mut pool = TexturePool::new(u64::MAX);
        let mut document = GpuDocument::new();
        let compositor = Compositor::new(&ctx).expect("shaders compile");
        document.sync(&ctx, &mut pool, &compositor, &project);

        let check = |document: &GpuDocument, pool: &TexturePool, channel, expected: [f32; 4]| {
            let set = document.texture_set(TextureSetId(0)).expect("synced");
            let texture = pool
                .peek(set.channel(channel).expect("active channel").texture)
                .expect("pinned");
            for level in 0..texture.mip_level_count() {
                if texture.format() == wgpu::TextureFormat::R16Float {
                    let texels = readback::read_r16f_level(&ctx, texture, level, 0).unwrap();
                    assert!(texels.iter().all(|t| *t == expected[0]), "{channel:?} mip {level}");
                } else {
                    let texels = readback::read_rgba16f_level(&ctx, texture, level, 0).unwrap();
                    assert!(texels.iter().all(|t| *t == expected), "{channel:?} mip {level}: {:?}", texels[0]);
                }
            }
        };
        check(&document, &pool, Channel::BaseColor, [0.5, 0.5, 0.5, 1.0]);
        check(&document, &pool, Channel::Roughness, [0.5, 0.0, 0.0, 0.0]);
        check(&document, &pool, Channel::Normal, [0.0, 0.0, 1.0, 0.0]);

        // Values exactly representable in f16.
        let mut red = ChannelDef::standard(Channel::BaseColor);
        red.default = [0.75, 0.125, 0.25, 1.0];
        let mut rough = ChannelDef::standard(Channel::Roughness);
        rough.default = [0.875, 0.0, 0.0, 0.0];
        patine_core::command::Command::apply(&mut SetChannelDef::set(TextureSetId(0), red), &mut project)
            .unwrap();
        patine_core::command::Command::apply(&mut SetChannelDef::set(TextureSetId(0), rough), &mut project)
            .unwrap();
        let memory = pool.memory();
        // The textures are reused (same bind groups): no texture set is reported.
        assert!(document.sync(&ctx, &mut pool, &compositor, &project).is_empty());
        assert_eq!(pool.memory(), memory, "a new default value reuses the textures");
        check(&document, &pool, Channel::BaseColor, [0.75, 0.125, 0.25, 1.0]);
        check(&document, &pool, Channel::Roughness, [0.875, 0.0, 0.0, 0.0]);
    }
);
