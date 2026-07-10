use criterion::{Criterion, criterion_group, criterion_main};

use macro_traffic_sim_core::trip_distribution::{FurnessConfig, furness_balance};

fn generate_furness_input(n: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut matrix = vec![0.0; n * n];
    let mut productions = vec![0.0; n];
    let mut attractions = vec![0.0; n];

    for i in 0..n {
        productions[i] = 100.0 + (i as f64 * 137.0) % 500.0;
        attractions[i] = 100.0 + (i as f64 * 271.0) % 500.0;
    }

    let attr_sum: f64 = attractions.iter().sum();
    let prod_sum: f64 = productions.iter().sum();
    let scale = prod_sum / attr_sum;
    for a in attractions.iter_mut() {
        *a *= scale;
    }

    for i in 0..n {
        for j in 0..n {
            if i != j {
                let dist = ((i as f64 - j as f64).abs() + 1.0).recip();
                matrix[i * n + j] = productions[i] * attractions[j] * dist;
            }
        }
    }

    (matrix, productions, attractions)
}

fn bench_furness_500(c: &mut Criterion) {
    let n = 500;
    let (matrix_template, productions, attractions) = generate_furness_input(n);
    let config = FurnessConfig {
        max_iterations: 100,
        tolerance: 1e-6,
    };

    c.bench_function("furness_500_zones", |b| {
        b.iter_batched(
            || matrix_template.clone(),
            |mut matrix| {
                furness_balance(&mut matrix, n, &productions, &attractions, &config).unwrap()
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn bench_furness_1000(c: &mut Criterion) {
    let n = 1000;
    let (matrix_template, productions, attractions) = generate_furness_input(n);
    let config = FurnessConfig {
        max_iterations: 100,
        tolerance: 1e-6,
    };

    c.bench_function("furness_1000_zones", |b| {
        b.iter_batched(
            || matrix_template.clone(),
            |mut matrix| {
                furness_balance(&mut matrix, n, &productions, &attractions, &config).unwrap()
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn bench_furness_2000(c: &mut Criterion) {
    let n = 2000;
    let (matrix_template, productions, attractions) = generate_furness_input(n);
    let config = FurnessConfig {
        max_iterations: 100,
        tolerance: 1e-6,
    };

    c.bench_function("furness_2000_zones", |b| {
        b.iter_batched(
            || matrix_template.clone(),
            |mut matrix| {
                furness_balance(&mut matrix, n, &productions, &attractions, &config).unwrap()
            },
            criterion::BatchSize::LargeInput,
        );
    });
}

criterion_group!(benches, bench_furness_500, bench_furness_1000, bench_furness_2000);
criterion_main!(benches);
