//! Benchmarks for file-picker scan performance.

use super::*;
use test_utils::TempTree;
use tiny_bench::black_box;

/// Advance one deterministic pseudo-random generator state.
fn advance_seed(seed: &mut u64) -> u64 {
    // The LCG constants keep output deterministic across platforms.
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    *seed
}

/// Return one pseudo-random lowercase ASCII token with fixed `length`.
fn random_token(seed: &mut u64, length: usize) -> String {
    let mut token = String::with_capacity(length);
    for _ in 0..length {
        let random_value = advance_seed(seed);
        let character = b'a' + ((random_value >> 32) % 26) as u8;
        token.push(character as char);
    }
    token
}

/// Build one deterministic pseudo-random fixture tree with configurable dimensions.
fn build_random_fixture_tree(
    tree: &TempTree,
    depth_one_count: usize,
    depth_two_count: usize,
    files_per_directory: usize,
) {
    let mut seed = 0x5EED_1A2B_3C4D_5E6Fu64;
    for depth_one in 0..depth_one_count {
        for depth_two in 0..depth_two_count {
            for file_index in 0..files_per_directory {
                // Three-level trees stress directory recursion and per-file matching.
                let level_one = random_token(&mut seed, 6);
                let level_two = random_token(&mut seed, 5);
                let file_stem = random_token(&mut seed, 8);
                let extension = if advance_seed(&mut seed) & 1 == 0 {
                    "rs"
                } else {
                    "txt"
                };
                let file_path = format!(
                    "bench/{level_one}_{depth_one:03}/{level_two}_{depth_two:02}/{file_stem}_{file_index:02}.{extension}"
                );
                tree.write_file(&file_path, "bench fixture\n")
                    .expect("write benchmark fixture file");
            }
        }
    }
    // Mix visible and ignored content so matcher hot paths are exercised.
    tree.write_file(".gitignore", "bench/**/target/\nbench/**/tmp/\n")
        .expect("write benchmark gitignore");
    tree.write_file(".ignore", "!bench/**/tmp/keep.txt\n")
        .expect("write benchmark picker ignore");
    tree.write_file("bench/aa_ignore/target/output.o", "ignored\n")
        .expect("write ignored target artifact");
    tree.write_file("bench/ab_ignore/tmp/keep.txt", "keep\n")
        .expect("write reincluded keep file");
}

/// Build one deterministic pseudo-random fixture tree sized for sub-40s benchmark runs.
fn build_medium_random_fixture_tree(tree: &TempTree) {
    build_random_fixture_tree(tree, 32, 4, 20);
}

/// Build one deterministic pseudo-random fixture tree sized for quick iteration runs.
fn build_small_random_fixture_tree(tree: &TempTree) {
    build_random_fixture_tree(tree, 18, 3, 14);
}

/// Benchmark file-picker scans on one deterministic pseudo-random tree.
#[test]
#[ignore = "manual benchmark"]
fn bench_scan_git_large_random_tree() {
    let tree = TempTree::new().expect("create temp tree");
    build_medium_random_fixture_tree(&tree);
    tiny_bench::bench_labeled("file_picker_scan_git_large_random_tree", || {
        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan picker tree")
        .expect("picker scan summary");
        let mut emitted_paths = 0usize;
        while let Ok(event) = receiver.try_recv() {
            // Consume every streamed batch so each iteration performs full scan work.
            if let FilePickerEvent::Batch(batch) = event {
                emitted_paths += batch.len();
            }
        }
        black_box(summary);
        black_box(emitted_paths);
    });
}

/// Benchmark file-picker scans on one smaller tree for fast regression checks.
#[test]
#[ignore = "manual benchmark"]
fn bench_scan_git_small_random_tree() {
    let tree = TempTree::new().expect("create temp tree");
    build_small_random_fixture_tree(&tree);
    tiny_bench::bench_labeled("file_picker_scan_git_small_random_tree", || {
        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan picker tree")
        .expect("picker scan summary");
        let mut emitted_paths = 0usize;
        while let Ok(event) = receiver.try_recv() {
            // Consume every streamed batch so each iteration performs full scan work.
            if let FilePickerEvent::Batch(batch) = event {
                emitted_paths += batch.len();
            }
        }
        black_box(summary);
        black_box(emitted_paths);
    });
}
