#[derive(PartialEq, Eq, Hash, PartialOrd, Ord, Clone, Copy, Debug)]
pub enum DataMode {
    Single,
    Multi,
}

#[derive(PartialEq, Eq, Hash, PartialOrd, Ord, Clone, Copy, Debug)]
pub enum PlotType {
    Scatter,
    PerformanceProfile,
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum ProfileFilter {
    None,
    MaxTau(f64),
    TrimPercent(f64),
}

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
