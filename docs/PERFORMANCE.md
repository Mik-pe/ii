# Initial performance measurements

These are microbenchmarks, not a startup or keypress-latency guarantee.

Measured on 2026-10-07 in a GitHub-hosted Ubuntu 24.04 runner using Rust 1.99.0 and `cargo bench --locked --bench navigation` (optimized bench profile). The filesystem was warm. Different machines, storage, mount types, and concurrent workloads will produce different results; the runner's precise CPU model was not recorded.

| Workload | Samples | Median | p95 |
| --- | ---: | ---: | ---: |
| Scan 500 directories plus 4,500 ordinary files in one directory | 30 | 2.243 ms | 2.273 ms |
| Subsequence-score 10,000 preconstructed directory names | 100 | 0.269 ms | 0.308 ms |

[Source run and benchmark logs](https://github.com/Mik-pe/ii/actions/runs/37586578634). The code tested was committed as `7d2aab669a7c988ceb8bfb5c90ff0ea881d748ff` during the initialization job; that job checked and built the working tree after applying and committing compiler-suggested cleanup.

The scoring measurement does not include sorting matches, rendering, reading the filesystem, or starting the binary. The scan includes listing, selecting directory entries, label construction, and sorting. There is no comparison with other navigators.

To reproduce the workloads:

```sh
cargo bench --locked --bench navigation
```

For future work, separately measure process startup, input-to-render latency through a PTY, cold-cache filesystem behavior, larger directories, slow network mounts, and allocation counts. Do not replace these targeted measurements with a universal "blazing fast" claim.
