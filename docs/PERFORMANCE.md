# Performance measurements

These are microbenchmarks, not a startup or keypress-latency guarantee. No comparison with other navigators has been measured.

## Mixed file and directory listing

Measured on 2026-10-07 on a GitHub-hosted Ubuntu 24.04 runner with Rust 1.99.0, using `cargo bench --locked --bench navigation` (optimized bench profile). The filesystem was warm. The precise CPU model was not recorded.

| Workload | Samples | Median | p95 |
| --- | ---: | ---: | ---: |
| Scan and classify 500 directories plus 4,500 files, retaining all 5,000 entries | 30 | 4.309 ms | 4.655 ms |
| Subsequence-score 10,000 preconstructed names | 100 | 0.269 ms | 0.281 ms |

[Source run and benchmark logs](https://github.com/Mik-pe/ii/actions/runs/37591953346). The validated tree was committed as `3e2a55063f86caba3286411c7f3b4effbc4f6550` during the preparation job after generating the documentation SVG. The runtime code was unchanged by that asset-only commit.

The scan includes listing, directory/file classification, label construction, and directory-first sorting. It does not read file contents or obtain sizes, timestamps, or executable permissions. Only symlinks need a target-type metadata check. The benchmark fixture uses ordinary files/directories, not symlinks or network mounts.

The scoring measurement excludes sorting matches, rendering, scanning, and starting the process. Debug benchmark output from `cargo test --all-targets` is not used in this table.

Showing files constructs and sorts more entries than ignoring them. `--dirs-only` changes visibility using the same all-entry listing/cache, so it should not be described as restoring the old directory-only scan cost.

## Historical directory-only baseline

Before file visibility, the scan retained only 500 directories from the same 5,000-entry workload. A separate Ubuntu runner measured median 2.243 ms / p95 2.273 ms, with 30 samples. [Historical run](https://github.com/Mik-pe/ii/actions/runs/37586578634), commit `7d2aab669a7c988ceb8bfb5c90ff0ea881d748ff`.

That result is not the current all-entry workload and was not a controlled same-machine comparison. Do not use it as the current implementation's listing time or infer an exact regression percentage between separate hosted runners.

## Reproduce

```sh
cargo bench --locked --bench navigation
```

Results depend on hardware, storage, mounts, caches, and concurrent workloads. Future measurements should separately cover process startup, input-to-render latency through a PTY, cold filesystems, larger directories, slow network mounts, full filter-and-sort behavior, and allocations. Do not replace these targeted measurements with a universal speed claim.
