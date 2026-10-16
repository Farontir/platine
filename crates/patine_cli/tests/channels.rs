//! End to end: `patine_cli channels` writes every channel of a saved project at the
//! requested resolution (software adapter).

use std::process::Command;

use patine_core::channel::Channel;
use patine_core::command::Command as _;
use patine_core::commands::InsertLayer;
use patine_core::file;
use patine_core::ids::IdSequence;
use patine_core::layer::{FillSource, Layer, LayerKind, LayerPosition, Projection};
use patine_core::project::{Project, ProjectSettings, Resolution, TextureSetId};
use patine_mesh::primitives;

#[test]
fn channels_are_written_at_the_requested_resolution() {
    let dir = std::env::temp_dir().join(format!("patine-channels-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temporary folder");

    let settings = ProjectSettings {
        resolution: Resolution::new(512).expect("supported"),
        ..ProjectSettings::default()
    };
    let mut project = Project::new(primitives::cube_cross(), "cube.glb".into(), settings);
    let mut ids = IdSequence::starting_at(project.next_id);
    let channels: Vec<_> = project.texture_sets[0].channels.clone();
    let mut fill = Layer::fill_for(ids.layer(), "fill", &channels);
    fill.kind = LayerKind::Fill {
        source: FillSource::Uniform(
            [
                (Channel::BaseColor, [0.5, 0.25, 1.0, 1.0]),
                (Channel::Roughness, [0.75; 4]),
            ]
            .into(),
        ),
        projection: Projection::default(),
    };
    InsertLayer::new(
        TextureSetId(0),
        LayerPosition {
            parent: None,
            index: 0,
        },
        fill,
    )
    .apply(&mut project)
    .expect("insert");
    project.next_id = ids.next_raw();
    let path = dir.join("cube.patine");
    file::save(&project, &path, None).expect("saved");

    let out = dir.join("out");
    let status = Command::new(env!("CARGO_BIN_EXE_patine_cli"))
        .args(["channels"])
        .arg(&path)
        .arg("--out")
        .arg(&out)
        .args(["--resolution", "1024", "--software"])
        .status()
        .expect("patine_cli runs");
    assert!(status.success());

    let set = &project.texture_sets[0].name;
    for def in &project.texture_sets[0].channels {
        let file = out.join(format!("{set}_{}.png", def.channel.id()));
        let image = image::open(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        assert_eq!(
            (image.width(), image.height()),
            (1024, 1024),
            "{}",
            file.display()
        );
    }
    let color = image::open(out.join(format!("{set}_base-color.png")))
        .expect("readable")
        .into_rgb16();
    let center = color.get_pixel(512, 512).0;
    // 0.5 linear is 0.7354 in sRGB; 0.25 is 0.5371.
    let expected = [0.735_357f32, 0.537_099, 1.0].map(|v| (v * 65535.0).round() as i32);
    for (got, want) in center.iter().zip(expected) {
        assert!(
            (i32::from(*got) - want).abs() <= 80,
            "{center:?} against {expected:?}"
        );
    }
    let roughness = image::open(out.join(format!("{set}_roughness.png")))
        .expect("readable")
        .into_luma16();
    let value = i32::from(roughness.get_pixel(512, 512).0[0]);
    assert!((value - 49151).abs() <= 40, "roughness {value}");
    let _ = std::fs::remove_dir_all(&dir);
}
