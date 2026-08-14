#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeSpec {
    Proportion(f64),
    Fixed(f64),
}

impl SizeSpec {
    pub fn to_niri_size_change(self) -> Option<niri_ipc::SizeChange> {
        match self {
            Self::Proportion(value) if value.is_finite() && value > 0.0 => {
                let percentage = value * 100.0;
                percentage
                    .is_finite()
                    .then_some(niri_ipc::SizeChange::SetProportion(percentage))
            }
            Self::Fixed(value)
                if value.is_finite() && value > 0.0 && value <= f64::from(i32::MAX) =>
            {
                Some(niri_ipc::SizeChange::SetFixed(value.round().max(1.0) as i32))
            }
            _ => None,
        }
    }

    pub fn is_full_proportion(self) -> bool {
        matches!(self, Self::Proportion(value) if (value - 1.0).abs() < 0.005)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnWidthSpec {
    /// Fraction of the Niri working area, where `1.0` is 100%.
    Proportion(f64),
    /// Width in logical pixels.
    Fixed(f64),
}

impl From<ColumnWidthSpec> for SizeSpec {
    fn from(value: ColumnWidthSpec) -> Self {
        match value {
            ColumnWidthSpec::Proportion(value) => Self::Proportion(value),
            ColumnWidthSpec::Fixed(value) => Self::Fixed(value),
        }
    }
}

pub fn column_width_percentage(tile_width: f64, output_width: Option<f64>) -> u32 {
    let Some(output_width) = output_width.filter(|width| *width > 0.0) else {
        return 50;
    };
    if tile_width <= 0.0 {
        return 50;
    }
    let percentage = (tile_width / output_width * 100.0).ceil() as u32;
    if percentage >= 98 {
        100
    } else {
        percentage.clamp(10, 100)
    }
}

pub fn inferred_working_extent(logical_extent: Option<u32>, observed_extent: f64) -> Option<f64> {
    let logical_extent = logical_extent.filter(|extent| *extent > 0).map(f64::from);
    let observed_extent =
        (observed_extent.is_finite() && observed_extent > 0.0).then_some(observed_extent);

    match (logical_extent, observed_extent) {
        (Some(logical), Some(observed)) if (0.9..=1.1).contains(&(observed / logical)) => {
            Some(observed)
        }
        (Some(logical), _) => Some(logical),
        (None, observed) => observed,
    }
}

pub fn infer_proportional_size(value: f64, working_extent: Option<f64>) -> SizeSpec {
    let proportion = working_extent
        .filter(|extent| extent.is_finite() && *extent > 0.0)
        .map(|extent| value / extent)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);
    SizeSpec::Proportion(((proportion * 100.0).round() / 100.0).max(0.01))
}

pub fn snapshot_size(
    known: Option<SizeSpec>,
    floating: bool,
    observed: f64,
    working_extent: Option<f64>,
) -> SizeSpec {
    let observed = if observed.is_finite() && observed > 0.0 {
        observed
    } else {
        1.0
    };
    known.unwrap_or_else(|| {
        if floating {
            SizeSpec::Fixed(observed)
        } else {
            infer_proportional_size(observed, working_extent)
        }
    })
}

pub fn is_significant_extent_change(previous: f64, current: f64) -> bool {
    previous.is_finite()
        && current.is_finite()
        && previous > 0.0
        && ((current - previous).abs() / previous) >= 0.2
}

#[cfg(test)]
mod tests {
    use super::{
        SizeSpec, column_width_percentage, infer_proportional_size, inferred_working_extent,
        is_significant_extent_change, snapshot_size,
    };

    #[test]
    fn converts_fraction_to_niri_percentage() {
        assert_eq!(
            SizeSpec::Proportion(0.4).to_niri_size_change(),
            Some(niri_ipc::SizeChange::SetProportion(40.0))
        );
        assert_eq!(
            SizeSpec::Fixed(754.0).to_niri_size_change(),
            Some(niri_ipc::SizeChange::SetFixed(754))
        );
        assert!(SizeSpec::Proportion(1.0).is_full_proportion());
        assert!(!SizeSpec::Proportion(0.5).is_full_proportion());
    }

    #[test]
    fn calculates_column_width_percentage() {
        assert_eq!(column_width_percentage(960.0, Some(1920.0)), 50);
        assert_eq!(column_width_percentage(1900.0, Some(1920.0)), 100);
        assert_eq!(column_width_percentage(0.0, Some(1920.0)), 50);
    }

    #[test]
    fn infers_working_area_and_common_proportions() {
        let width = inferred_working_extent(Some(1920), 1900.0);
        let height = inferred_working_extent(Some(1080), 1010.0);
        assert_eq!(width, Some(1900.0));
        assert_eq!(height, Some(1010.0));
        assert_eq!(
            infer_proportional_size(754.0, width),
            SizeSpec::Proportion(0.4)
        );
        assert_eq!(
            infer_proportional_size(945.0, width),
            SizeSpec::Proportion(0.5)
        );
        assert_eq!(
            infer_proportional_size(1900.0, width),
            SizeSpec::Proportion(1.0)
        );
        assert_eq!(
            infer_proportional_size(1010.0, height),
            SizeSpec::Proportion(1.0)
        );
    }

    #[test]
    fn defaults_tiled_sizes_to_proportional_and_floating_sizes_to_fixed() {
        assert_eq!(
            snapshot_size(None, false, 960.0, Some(1920.0)),
            SizeSpec::Proportion(0.5)
        );
        assert_eq!(
            snapshot_size(None, true, 960.0, Some(1920.0)),
            SizeSpec::Fixed(960.0)
        );
        assert_eq!(
            snapshot_size(Some(SizeSpec::Fixed(800.0)), false, 960.0, Some(1920.0)),
            SizeSpec::Fixed(800.0)
        );
    }

    #[test]
    fn detects_resolution_sized_extent_changes() {
        assert!(is_significant_extent_change(1900.0, 3820.0));
        assert!(is_significant_extent_change(3820.0, 1900.0));
        assert!(!is_significant_extent_change(1900.0, 1910.0));
    }
}
