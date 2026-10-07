//! Run `cargo bench --bench navigation`. Results describe this machine, not a universal claim.
use ii::{filesystem, filter};
use std::fs;
use std::hint::black_box;
use std::time::Instant;

fn measure(name: &str, runs: usize, mut operation: impl FnMut()) {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        operation();
        samples.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name}: median {:.3} ms · p95 {:.3} ms ({runs} samples)",
        samples[runs / 2],
        samples[((runs as f64 * 0.95) as usize).min(runs - 1)]
    );
}

fn main() {
    let root = tempfile::tempdir().expect("temporary benchmark directory");
    for index in 0..500 {
        fs::create_dir(root.path().join(format!("directory-{index:05}"))).unwrap();
    }
    for index in 0..4_500 {
        fs::write(root.path().join(format!("file-{index:05}")), []).unwrap();
    }
    measure(
        "scan 500 directories + 4500 files (warm filesystem)",
        30,
        || {
            black_box(filesystem::scan(black_box(root.path())).unwrap());
        },
    );
    let names: Vec<_> = (0..10_000)
        .map(|index| format!("project-{index:05}-source"))
        .collect();
    measure("subsequence-score 10,000 names", 100, || {
        let count = names
            .iter()
            .filter(|name| filter::score(black_box(name), black_box("psrc")).is_some())
            .count();
        black_box(count);
    });
}
