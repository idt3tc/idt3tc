//! CMP_Core initializes lookup tables lazily on first use. This test lives
//! in its own binary (= process) so that the very first block compressions
//! happen concurrently, which is what parallel batch processing of several
//! inputs does.

mod common;

use common::make_test_image;
use idt3tc::img::bcn::{BlockFormat, BlockOptions, compress_surface};
use idt3tc::img::image::to_rgba8;

#[test]
fn concurrent_first_use_is_deterministic() {
    let rgba = to_rgba8(&make_test_image(64, 64, true));
    let formats = [BlockFormat::Bc7, BlockFormat::Bc1, BlockFormat::Bc7, BlockFormat::Bc3];
    let compress = |f: BlockFormat, quality: f32| {
        let o = BlockOptions { quality, ..Default::default() };
        compress_surface(&rgba, 64, 64, f, &o).unwrap()
    };
    // Start everything at once on a fresh process.
    let barrier = std::sync::Barrier::new(16);
    let concurrent: Vec<(usize, Vec<u8>)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let (barrier, compress) = (&barrier, &compress);
                s.spawn(move || {
                    barrier.wait();
                    (i, compress(formats[i % 4], 0.05 + 0.05 * (i % 3) as f32))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (i, bytes) in concurrent {
        assert!(bytes == compress(formats[i % 4], 0.05 + 0.05 * (i % 3) as f32), "task {i}: {:?}", formats[i % 4]);
    }
}
