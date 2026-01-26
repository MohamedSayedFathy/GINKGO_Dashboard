use crate::types::MetricType;

#[test]
fn test_normalization_time() {
    let val = crate::visualization::math::calculate_normalized_value(5.0, 10.0, &MetricType::Time);
    assert_eq!(val, 2.0);
}

#[test]
fn test_normalization_gflops() {
    let val = crate::visualization::math::calculate_normalized_value(
        20.0,
        10.0,
        &MetricType::GflopsPerSecond,
    );
    assert_eq!(val, 2.0);
}

#[test]
fn test_outlier_logic_indices() {
    // Verify index logic for 5-95th percentile
    let len = 20;
    let p5_idx = (len as f64 * 0.05) as usize;
    let p95_idx = (len as f64 * 0.95) as usize;
    assert_eq!(p5_idx, 1);
    assert_eq!(p95_idx, 19);
}

#[test]
fn test_profile_counts_failures() {
    use crate::data::models::{
        BenchmarkDataset, BenchmarkEntry, BenchmarkOptimal, BenchmarkProblem,
        MatrixColumnsMetadata, MatrixMetadata, MatrixRowsMetadata,
    };
    use crate::types::{DataFormat, MetricType, ProfileFilter};
    use crate::visualization::profile::generate_performance_profile_data;
    use std::collections::HashMap;

    let dummy_dist = MatrixRowsMetadata {
        min: 0.,
        q1: 0.,
        median: 0.,
        q3: 0.,
        max: 0.,
        mean: 0.,
        variance: 0.,
        skewness: 0.,
        kurtosis: 0.,
        hyperskewness: 0.,
        hyperflatness: 0.,
        row_irregularity: 0.,
        row_cv: 0.,
    };
    let dummy_col_dist = MatrixColumnsMetadata {
        min: 0.,
        q1: 0.,
        median: 0.,
        q3: 0.,
        max: 0.,
        mean: 0.,
        variance: 0.,
        skewness: 0.,
        kurtosis: 0.,
        hyperskewness: 0.,
        hyperflatness: 0.,
        col_irregularity: 0.,
        col_cv: 0.,
    };
    let meta = MatrixMetadata {
        id: 1,
        group: "g".into(),
        name: "A".into(),
        rows: 10,
        cols: 10,
        nonzeros: 10,
        real: true,
        binary: false,
        is_2d3d: false,
        posdef: false,
        psym: 0.,
        nsym: 0.,
        kind: "k".into(),
        row_distribution: dummy_dist.clone(),
        col_distribution: dummy_col_dist.clone(),
        sparsity: 0.1,
        avg_nnz_per_row: 1.0,
        avg_nnz_per_col: 1.0,
        matrix_shape_ratio: 1.0,
    };

    // Problem A: Success
    let mut spmv_a = HashMap::new();
    spmv_a.insert(
        "csr".to_string(),
        BenchmarkEntry {
            time: Some(1.0),
            storage: Some(1),
            max_relative_norm2: None,
            repetitions: None,
            completed: true,
            gflops_per_second: 1.0,
            bytes_per_nnz: 1.0,
            operational_intensity: 1.0,
            effective_memory_bandwidth: 1.0,
        },
    );

    let prob_a = BenchmarkProblem {
        filename: "A".to_string(),
        problem: meta.clone(),
        spmv: spmv_a,
        optimal: BenchmarkOptimal {
            spmv: "csr".to_string(),
        },
    };

    // Problem B: Failure (Empty SpMV map or missing "csr")
    let mut meta_b = meta.clone();
    meta_b.name = "B".into();
    let prob_b = BenchmarkProblem {
        filename: "B".to_string(),
        problem: meta_b,
        spmv: HashMap::new(), // No data
        optimal: BenchmarkOptimal {
            spmv: "csr".to_string(),
        },
    };

    let ds = BenchmarkDataset {
        benchmark: vec![prob_a, prob_b],
    };
    let mut datasets = HashMap::new();
    datasets.insert("DS1".to_string(), ds);

    let active_ds = vec!["DS1".to_string()];
    let mut active_fmt = HashMap::new();
    active_fmt.insert(DataFormat::CSR, true);

    let plot_data = generate_performance_profile_data(
        &datasets,
        &active_ds,
        &active_fmt,
        Some(crate::types::DataMode::Multi),
        Some(MetricType::Time),
        ProfileFilter::None,
        false, // log scale
    );

    // Check Series 0
    if plot_data.series.is_empty() {
        panic!("Series should not be empty");
    }
    let series = &plot_data.series[0];

    // Series should contain points for the 1 successful problem.
    // x=1.0. y should be count/total.
    // If current logic ignores failures, total=1, so y=1/1=1.0.
    // If correct logic counts failures, total=2, so y=1/2=0.5.
    let last_point = series.points.last().unwrap();
    assert_eq!(
        last_point[1], 0.5,
        "Expected Y=0.5 (1/2), got {}",
        last_point[1]
    );
}
