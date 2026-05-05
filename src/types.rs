#[derive(PartialEq, Eq, Hash, PartialOrd, Ord, Clone, Copy, Debug)]
pub enum DataMode {
    Single,
    Multi,
}

#[derive(PartialEq, Eq, Hash, PartialOrd, Ord, Clone, Copy, Debug)]
pub enum PlotType {
    Scatter,
    PerformanceProfile,
    Comparison,
    /// Trend a single format's aggregated metric across every loaded dataset,
    /// ordered by commit date when a git repo is loaded.
    LineTimeseries,
    /// Per-problem storage-format breakdown within a single dataset.
    StackedBar,
}

/// Scalar aggregation kernel applied over a dataset's problems when reducing
/// them to one point on the line timeseries plot.
///
/// `GeometricMean` rejects non-positive values (they're undefined in log
/// space) and reports the skip count via `log::debug` rather than returning
/// silently-nan outputs.
#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum AggregationKind {
    Mean,
    Median,
    GeometricMean,
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum ProfileFilter {
    None,
    MaxTau(f64),
    TrimPercent(f64),
}

// CSR/COO/ELL/HYBRID/SELLP are standard sparse-matrix format acronyms in HPC
// literature; `Csr`/`Coo` etc. would read worse for the domain.
#[allow(clippy::upper_case_acronyms)]
#[derive(PartialEq, Eq, Hash, PartialOrd, Ord, Clone, Copy, Debug)]
pub enum DataFormat {
    CSR,
    COO,
    ELL,
    HYBRID,
    SELLP,
}

impl DataFormat {
    /// Returns the JSON key string for this format
    pub fn as_key(&self) -> &'static str {
        match self {
            DataFormat::CSR => "csr",
            DataFormat::COO => "coo",
            DataFormat::ELL => "ell",
            DataFormat::HYBRID => "hybrid",
            DataFormat::SELLP => "sellp",
        }
    }
}

#[derive(PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Clone, Copy)]
pub enum MetricType {
    Storage,
    Time,
    GflopsPerSecond,
    Repetitions,
    OperationalIntensity,
    EffectiveMemoryBandwidth,
    Custom,
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum XaxisType {
    Cols,
    ColCv,
    Rows,
    RowCv,
    NonZeros,
    Sparsity,
    AvgNnzPerRow,
    AvgNnzPerCol,
    MatrixShapeRatio,
}
