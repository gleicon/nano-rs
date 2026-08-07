//! Cold Start Benchmark for Sliver Restoration
//!
//! Measures the time from sliver load to first request handling.
//! Compares sliver-based cold start against context reset and fresh isolate creation.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nano::sliver::{pack_sliver, unpack_sliver, SliverMetadata, UnpackedSliver};
use nano::vfs::{IsolateVfs, MemoryBackend, VfsFile, VfsNamespace, VfsPath};
use std::sync::Arc;

/// Create a test sliver with various sizes
fn create_test_sliver(file_count: usize) -> UnpackedSliver {
    let hostname = format!("bench{}.example.com", file_count);
    let metadata = SliverMetadata::new(&hostname, "1.1.0");

    // Create heap data (simulate ~1MB snapshot)
    let heap_data = vec![0xABu8; 1024 * 1024];

    // Create VFS entries
    let vfs_entries: Vec<(VfsPath, VfsFile)> = (0..file_count)
        .map(|i| {
            let path = VfsPath::new(&format!("data/file{}.txt", i)).unwrap();
            let content = format!("Content of file {}", i).into_bytes();
            let file = VfsFile::new(content);
            (path, file)
        })
        .collect();

    let archive =
        pack_sliver(&metadata, &heap_data, Some(&vfs_entries)).expect("Failed to pack sliver");

    unpack_sliver(&archive).expect("Failed to unpack sliver")
}

/// Benchmark sliver cold start (unpack + VFS restore)
fn bench_sliver_cold_start(c: &mut Criterion) {
    let mut group = c.benchmark_group("sliver_cold_start");

    for file_count in [0, 10, 50, 100].iter() {
        let unpacked = create_test_sliver(*file_count);

        group.throughput(Throughput::Elements(1));
        group.bench_with_input(
            BenchmarkId::new("unpack_and_restore", file_count),
            file_count,
            |b, _| {
                b.iter(|| {
                    // Clone the unpacked sliver for each iteration
                    let sliver = unpacked.clone();

                    // Create a fresh VFS
                    let backend = Arc::new(MemoryBackend::default());
                    let vfs =
                        IsolateVfs::new(VfsNamespace::from_hostname("bench.example.com"), backend);

                    // Restore VFS entries (this is what happens during cold start)
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    rt.block_on(async {
                        sliver.restore_to_vfs(&vfs).await.unwrap();
                    });

                    // Simulate V8 snapshot restoration check
                    // (In real scenario, this would create the isolate)
                    assert!(!sliver.heap_data.is_empty());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark just the VFS restoration (after initial unpack)
fn bench_vfs_restore_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("vfs_restore_only");

    for file_count in [0, 10, 50, 100].iter() {
        let unpacked = create_test_sliver(*file_count);

        group.throughput(Throughput::Elements(*file_count as u64));
        group.bench_with_input(
            BenchmarkId::new("restore_entries", file_count),
            file_count,
            |b, _| {
                // Pre-create the VFS
                let backend = Arc::new(MemoryBackend::default());
                let vfs =
                    IsolateVfs::new(VfsNamespace::from_hostname("bench.example.com"), backend);

                b.iter(|| {
                    let sliver = unpacked.clone();
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    rt.block_on(async {
                        sliver.restore_to_vfs(&vfs).await.unwrap();
                    });
                });
            },
        );
    }

    group.finish();
}

/// Benchmark tar unpacking (simulating sliver file read)
fn bench_sliver_unpack(c: &mut Criterion) {
    let mut group = c.benchmark_group("sliver_unpack");

    for file_count in [0, 10, 50, 100].iter() {
        let unpacked = create_test_sliver(*file_count);
        // Re-pack to get the archive bytes
        let archive = pack_sliver(
            &unpacked.metadata,
            &unpacked.heap_data,
            Some(&unpacked.vfs_entries),
        )
        .expect("Failed to re-pack sliver");

        group.throughput(Throughput::Bytes(archive.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("tar_unpack", file_count),
            &archive,
            |b, archive_data| {
                b.iter(|| {
                    let unpacked = unpack_sliver(archive_data).expect("Failed to unpack");
                    assert_eq!(
                        unpacked.metadata.hostname,
                        format!("bench{}.example.com", file_count)
                    );
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_sliver_cold_start,
    bench_vfs_restore_only,
    bench_sliver_unpack
);
criterion_main!(benches);
