use criterion::{criterion_group, criterion_main};

mod price_level;
mod simple;

// mod concurrent; // Disabled for single-threaded design

// use concurrent::register_benchmarks as register_concurrent_benchmarks; // Disabled for single-threaded design
use price_level::register_benchmarks as register_price_level_benchmarks;
use simple::first::benchmark_data;

// Define the benchmark groups
criterion_group!(
    benches,
    benchmark_data,
    register_price_level_benchmarks,
    // register_concurrent_benchmarks, // Disabled for single-threaded design
);

criterion_main!(benches);
