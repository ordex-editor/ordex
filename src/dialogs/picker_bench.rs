//! Benchmarks for picker streaming ingestion and merge behavior.

use super::*;
use tiny_bench::{BenchmarkConfig, black_box};

const GCC_BATCH_COUNT: usize = 2_472;
const GCC_BATCH_SIZE: usize = 64;
const NON_EMPTY_QUERY_MAX_ITERATIONS: u64 = 100;

/// One lightweight benchmark picker item with stable order and label text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BenchItem {
    label: String,
    order: usize,
}

impl PickerItem for BenchItem {
    /// Return one item label used for fuzzy scoring and popup rows.
    fn label(&self) -> &str {
        &self.label
    }

    /// Return one stable sort tie-breaker order for this benchmark item.
    fn order(&self) -> usize {
        self.order
    }

    /// Build one plain picker row for benchmark-only rendering compatibility.
    fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
        PickerPopupEntry {
            label: self.label.clone(),
            search_result_parts: None,
            selected,
            primary_marker: false,
            secondary_marker: false,
        }
    }
}

/// Build one deterministic batch list that mimics GCC-scale streaming discovery.
fn build_gcc_like_batches() -> Vec<Vec<BenchItem>> {
    let mut batches = Vec::with_capacity(GCC_BATCH_COUNT);
    let mut next_order = 0usize;
    for batch_index in 0..GCC_BATCH_COUNT {
        let mut batch = Vec::with_capacity(GCC_BATCH_SIZE);
        // The generated labels vary depth and basename tokens so merge and fuzzy
        // comparison paths observe realistic candidate diversity.
        for item_index in 0..GCC_BATCH_SIZE {
            let item = BenchItem {
                label: format!(
                    "gcc/{:04}/module_{:03}/file_{:05}.rs",
                    batch_index % 240,
                    batch_index % 97,
                    batch_index.saturating_mul(GCC_BATCH_SIZE) + item_index
                ),
                order: next_order,
            };
            next_order = next_order.saturating_add(1);
            batch.push(item);
        }
        batches.push(batch);
    }
    batches
}

/// Return one constrained tiny-bench configuration for the slow non-empty query benchmark.
fn non_empty_query_benchmark_config() -> BenchmarkConfig {
    BenchmarkConfig {
        max_iterations: Some(NON_EMPTY_QUERY_MAX_ITERATIONS),
        ..BenchmarkConfig::default()
    }
}

/// Benchmark one full empty-query streaming ingest using GCC-like batch counts.
#[test]
#[ignore = "manual benchmark"]
fn bench_picker_extend_items_empty_query_gcc_profile() {
    let batches = build_gcc_like_batches();
    tiny_bench::bench_labeled("picker_extend_items_empty_query_gcc_profile", || {
        let mut picker = PickerState::new(Vec::<BenchItem>::new());
        for batch in &batches {
            picker.extend_items(batch.iter().cloned(), "");
        }
        black_box(picker.item_count());
    });
}

/// Benchmark one non-empty query ingest to stress incremental scored merges.
#[test]
#[ignore = "manual benchmark"]
fn bench_picker_extend_items_non_empty_query_gcc_profile() {
    let batches = build_gcc_like_batches();
    tiny_bench::bench_with_configuration_labeled(
        "picker_extend_items_non_empty_query_gcc_profile",
        &non_empty_query_benchmark_config(),
        || {
            let mut picker = PickerState::new(Vec::<BenchItem>::new());
            // Keep one short query to exercise scored merge logic for every incoming batch.
            for batch in &batches {
                picker.extend_items(batch.iter().cloned(), "file");
            }
            black_box(picker.fuzzy_match_counts());
        },
    );
}
