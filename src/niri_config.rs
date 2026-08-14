use std::fs;

use crate::layout::ColumnWidthSpec;

#[derive(Debug, Clone)]
pub struct ColumnPreset {
    pub label: String,
    pub width: ColumnWidthSpec,
}

/// Reads `preset-column-widths` from the user's Niri configuration.
pub fn load_column_presets() -> Vec<ColumnPreset> {
    let mut presets = Vec::new();
    let mut possible_paths = Vec::new();
    if let Some(home_dir) = dirs::home_dir() {
        possible_paths.push(home_dir.join(".config/niri/config.kdl"));
    }
    if let Some(config_dir) = dirs::config_dir() {
        possible_paths.push(config_dir.join("niri/config.kdl"));
    }

    for path in possible_paths {
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        presets.extend(parse_column_presets(&content));
    }

    if presets.is_empty() {
        presets = [33, 50, 66]
            .into_iter()
            .map(|percentage| ColumnPreset {
                label: format!("{percentage}%"),
                width: ColumnWidthSpec::Proportion(f64::from(percentage) / 100.0),
            })
            .collect();
    }
    let mut unique = Vec::with_capacity(presets.len());
    for preset in presets {
        if !unique
            .iter()
            .any(|existing: &ColumnPreset| existing.label == preset.label)
        {
            unique.push(preset);
        }
    }
    log::debug!("Loaded {} column presets from niri config", unique.len());
    unique
}

fn parse_column_presets(content: &str) -> Vec<ColumnPreset> {
    let mut presets = Vec::new();
    let mut in_preset_block = false;
    for line in content.lines() {
        let trimmed = line.split("//").next().unwrap_or_default().trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains("preset-column-widths") {
            in_preset_block = true;
            continue;
        }
        if !in_preset_block {
            continue;
        }
        if trimmed.contains('}') {
            break;
        }
        let clean = trimmed.trim_matches(|character| matches!(character, ';' | '{' | '}'));
        let mut parts = clean.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some("proportion"), Some(value)) => {
                if let Ok(value) = value.trim_matches(';').parse::<f64>()
                    && value.is_finite()
                    && value > 0.0
                {
                    let percentage = (value * 100.0).round() as u32;
                    presets.push(ColumnPreset {
                        label: format!("{percentage}%"),
                        width: ColumnWidthSpec::Proportion(value),
                    });
                }
            }
            (Some("fixed"), Some(value)) => {
                if let Ok(value) = value.trim_matches(';').parse::<i32>()
                    && value > 0
                {
                    presets.push(ColumnPreset {
                        label: format!("{value}px"),
                        width: ColumnWidthSpec::Fixed(f64::from(value)),
                    });
                }
            }
            _ => {}
        }
    }
    presets
}

#[cfg(test)]
mod tests {
    use crate::layout::ColumnWidthSpec;

    use super::parse_column_presets;

    #[test]
    fn parses_fixed_and_proportional_presets() {
        let presets = parse_column_presets(
            "layout {\n preset-column-widths {\n proportion 0.5\n fixed 960\n }\n }",
        );
        assert_eq!(presets.len(), 2);
        assert_eq!(presets[0].width, ColumnWidthSpec::Proportion(0.5));
        assert_eq!(presets[1].width, ColumnWidthSpec::Fixed(960.0));
    }
}
