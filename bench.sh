# The workspace's benchmark harness is baumhard's alone
# (`lib/baumhard/benches/test_bench.rs`); TEST_CONVENTIONS §T2.3.
# Naming `-p mandala` here only bought a bench-profile rebuild of a
# crate with no bench targets.
cargo bench -p baumhard
