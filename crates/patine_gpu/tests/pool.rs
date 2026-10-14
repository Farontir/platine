//! Texture pool on the test adapter: accounting, LRU eviction, pinned textures, textures
//! kept until the flush that follows their last use.

use patine_gpu::pool::{Residency, TexturePool, allocated_bytes, texture_bytes};
use patine_gpu::{gpu_test, wgpu};

fn desc(size: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("pool test"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    }
}

gpu_test!(
    fn memory_is_counted_and_returned(ctx) {
        let mut pool = TexturePool::new(u64::MAX);
        let a = pool.allocate(&ctx, &desc(256), Residency::Pinned);
        let b = pool.allocate(&ctx, &desc(512), Residency::Discardable);
        assert_eq!(pool.memory(), texture_bytes(&desc(256)) + texture_bytes(&desc(512)));
        assert_eq!(pool.len(), 2);
        pool.release(a);
        pool.release(a); // Releasing twice is harmless.
        assert_eq!(pool.memory(), texture_bytes(&desc(512)));
        assert!(pool.get(a).is_none());
        assert!(pool.get(b).is_some());
        pool.release(b);
        assert_eq!(pool.memory(), 0);
        assert!(pool.is_empty());
    }
);

gpu_test!(
    /// Over budget, the least recently used discardable textures go first; pinned ones
    /// stay even when the budget cannot be met.
    fn eviction_follows_the_lru_order(ctx) {
        let one = texture_bytes(&desc(128));
        let mut pool = TexturePool::new(3 * one);
        let pinned = pool.allocate(&ctx, &desc(128), Residency::Pinned);
        let old = pool.allocate(&ctx, &desc(128), Residency::Discardable);
        let recent = pool.allocate(&ctx, &desc(128), Residency::Discardable);
        // Use the oldest discardable texture: the other one becomes the LRU. The flush
        // ends the recording that used them, so they become evictable.
        assert!(pool.get(old).is_some());
        pool.flush();
        let newest = pool.allocate(&ctx, &desc(128), Residency::Discardable);
        assert!(pool.get(recent).is_none(), "the least recently used is evicted");
        assert!(pool.get(old).is_some() && pool.get(newest).is_some());
        assert!(pool.get(pinned).is_some());
        assert_eq!(pool.memory(), 3 * one);
        assert_eq!(pool.evicted(), 1);

        // A tiny budget leaves only the pinned texture, once the textures used in the
        // current recording are flushed.
        pool.set_budget(1);
        assert!(pool.get(newest).is_some(), "used since the last flush");
        pool.flush();
        assert_eq!(pool.len(), 1);
        assert!(pool.get(pinned).is_some());
        assert_eq!(pool.memory(), one);
    }
);

gpu_test!(
    /// Released textures give their memory back to the device, as wgpu reports it.
    fn released_textures_free_device_memory(ctx) {
        let Some(before) = allocated_bytes(&ctx) else {
            eprintln!("the backend does not report allocations: pool accounting only");
            return;
        };
        let mut pool = TexturePool::new(u64::MAX);
        let ids: Vec<_> = (0..4)
            .map(|_| pool.allocate(&ctx, &desc(1024), Residency::Pinned))
            .collect();
        let during = allocated_bytes(&ctx).expect("reported before");
        assert!(
            during >= before + pool.memory(),
            "allocated {during}, expected at least {} + {}",
            before,
            pool.memory()
        );
        for id in ids {
            pool.release(id);
        }
        pool.flush();
        // Destruction is processed by the device at the next maintenance.
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        let after = allocated_bytes(&ctx).expect("reported before");
        // wgpu's own transient allocations move the total by a few hundred bytes; one
        // leaked 1024² texture would be 8 MB.
        assert!(
            after.abs_diff(before) < 64 * 1024,
            "allocated before {before}, after {after}"
        );
    }
);

gpu_test!(
    /// While recording, nothing allocated or used since the last flush is evicted, even
    /// over budget, and released textures are only destroyed by the flush: the commands
    /// recorded with them stay valid until their submit.
    fn textures_live_until_the_flush_after_their_last_use(ctx) {
        let one = texture_bytes(&desc(64));
        let mut pool = TexturePool::new(one);
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        let clear = |encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture| {
            let view = texture.create_view(&Default::default());
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pool test"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        };
        let a = pool.allocate(&ctx, &desc(64), Residency::Discardable);
        clear(&mut encoder, pool.peek(a).expect("allocated"));
        let b = pool.allocate(&ctx, &desc(64), Residency::Discardable);
        clear(&mut encoder, pool.peek(b).expect("allocated"));
        assert!(pool.get(a).is_some() && pool.get(b).is_some(), "busy textures stay");
        assert_eq!(pool.memory(), 2 * one, "over budget while recording");
        pool.release(b);
        assert_eq!(pool.memory(), one, "released memory leaves the accounting at once");
        assert_eq!(pool.pending(), 1);
        // The submit still finds both textures alive.
        ctx.queue.submit([encoder.finish()]);
        pool.flush();
        assert_eq!(pool.pending(), 0);
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());

        // After the flush, `a` is evictable again: a new allocation takes its place.
        let c = pool.allocate(&ctx, &desc(64), Residency::Discardable);
        assert!(pool.get(a).is_none(), "evicted once its recording was flushed");
        assert!(pool.get(c).is_some());
        assert_eq!(pool.memory(), one);
        assert_eq!(pool.evicted(), 1);
    }
);
