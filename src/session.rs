use kdl::{KdlDocument, KdlNode, KdlValue};
use std::path::{Path, PathBuf};

use crate::layout::SizeSpec;

pub const SESSION_VERSION: i128 = 2;
const MAX_SESSION_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct OutputSnapshot {
    pub name: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub workspaces: Vec<WorkspaceSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceSnapshot {
    pub id: u64,
    pub name: Option<String>,
    pub index: u8,
    pub windows: Vec<WindowSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowSnapshot {
    pub app_id: String,
    pub column: usize,
    pub tile: usize,
    pub floating: bool,
    pub focused: bool,
    pub width: SizeSpec,
    pub height: SizeSpec,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionSnapshot {
    pub outputs: Vec<OutputSnapshot>,
}

pub fn session_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("cosmic-ext-niri-windows").join("session.kdl"))
}

pub fn load_from(path: &Path) -> Result<Option<SessionSnapshot>, String> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to inspect session {:?}: {error}", path)),
    };
    if metadata.len() > MAX_SESSION_BYTES {
        return Err("session file exceeds the maximum supported size".to_string());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read session {:?}: {error}", path))?;
    parse(&content).map(Some)
}

pub fn legacy_enabled(path: &Path) -> Option<bool> {
    let content = std::fs::read_to_string(path).ok()?;
    let document: KdlDocument = content.parse().ok()?;
    document
        .get("session")
        .and_then(|node| node.entry("enabled"))
        .and_then(|entry| entry.value().as_bool())
}

pub async fn save_to(path: &Path, snapshot: SessionSnapshot) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "session config has no parent directory".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("failed to create session directory: {error}"))?;

    let temporary = path.with_extension("kdl.tmp");
    tokio::fs::write(&temporary, serialize(&snapshot))
        .await
        .map_err(|error| format!("failed to write temporary session: {error}"))?;
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|error| format!("failed to commit session file: {error}"))
}

pub fn serialize(snapshot: &SessionSnapshot) -> String {
    let mut document = KdlDocument::new();
    document.nodes_mut().push({
        let mut version = KdlNode::new("format-version");
        version.push(SESSION_VERSION);
        version
    });

    for output in &snapshot.outputs {
        let mut output_node = KdlNode::new("output");
        if let Some(name) = &output.name {
            output_node.insert("name", name.clone());
        }
        if let Some(width) = output.width {
            output_node.insert("w", width as i128);
        }
        if let Some(height) = output.height {
            output_node.insert("h", height as i128);
        }
        let output_children = output_node.ensure_children();
        for workspace in &output.workspaces {
            let mut workspace_node = KdlNode::new("workspace");
            workspace_node.insert("id", workspace.id as i128);
            workspace_node.insert("idx", workspace.index as i128);
            if let Some(name) = &workspace.name {
                workspace_node.insert("name", name.clone());
            }
            let workspace_children = workspace_node.ensure_children();
            for window in &workspace.windows {
                let mut window_node = KdlNode::new("window");
                window_node.insert("app", window.app_id.clone());
                window_node.insert("c", window.column as i128);
                window_node.insert("t", window.tile as i128);
                window_node.insert("f", window.floating);
                window_node.insert("focused", window.focused);
                insert_size(&mut window_node, window.width, "wp", "wf");
                insert_size(&mut window_node, window.height, "hp", "hf");
                workspace_children.nodes_mut().push(window_node);
            }
            output_children.nodes_mut().push(workspace_node);
        }
        document.nodes_mut().push(output_node);
    }
    document.to_string()
}

pub fn parse(content: &str) -> Result<SessionSnapshot, String> {
    let document: KdlDocument = content
        .parse()
        .map_err(|error| format!("invalid session KDL: {error}"))?;
    if document
        .get_arg("format-version")
        .and_then(KdlValue::as_integer)
        != Some(SESSION_VERSION)
    {
        return Err("unsupported session version".to_string());
    }

    let mut snapshot = SessionSnapshot::default();
    for output_node in document.nodes() {
        if output_node.name().value() != "output" {
            continue;
        }
        let mut output = OutputSnapshot {
            name: property_string(output_node, "name"),
            width: property_u32(output_node, "w")?,
            height: property_u32(output_node, "h")?,
            workspaces: Vec::new(),
        };
        let Some(output_children) = output_node.children() else {
            snapshot.outputs.push(output);
            continue;
        };
        for workspace_node in output_children.nodes() {
            if workspace_node.name().value() != "workspace" {
                continue;
            }
            let mut workspace = WorkspaceSnapshot {
                id: required_u64(workspace_node, "id")?,
                name: property_string(workspace_node, "name"),
                index: required_u8(workspace_node, "idx")?,
                windows: Vec::new(),
            };
            if let Some(workspace_children) = workspace_node.children() {
                for window_node in workspace_children.nodes() {
                    if window_node.name().value() != "window" {
                        continue;
                    }
                    workspace.windows.push(WindowSnapshot {
                        app_id: required_string(window_node, "app")?,
                        column: required_usize(window_node, "c")?,
                        tile: required_usize(window_node, "t")?,
                        floating: property_bool(window_node, "f").unwrap_or(false),
                        focused: property_bool(window_node, "focused").unwrap_or(false),
                        width: required_size(window_node, "wp", "wf")?,
                        height: required_size(window_node, "hp", "hf")?,
                    });
                }
            }
            output.workspaces.push(workspace);
        }
        snapshot.outputs.push(output);
    }
    Ok(snapshot)
}

fn property_value<'a>(node: &'a KdlNode, name: &str) -> Option<&'a KdlValue> {
    node.entry(name).map(|entry| entry.value())
}

fn property_string(node: &KdlNode, name: &str) -> Option<String> {
    property_value(node, name)
        .and_then(KdlValue::as_string)
        .map(ToOwned::to_owned)
}

fn required_string(node: &KdlNode, name: &str) -> Result<String, String> {
    property_string(node, name).ok_or_else(|| format!("session property '{name}' is missing"))
}

fn property_integer(node: &KdlNode, name: &str) -> Option<i128> {
    property_value(node, name).and_then(KdlValue::as_integer)
}

fn required_u64(node: &KdlNode, name: &str) -> Result<u64, String> {
    property_integer(node, name)
        .ok_or_else(|| format!("session property '{name}' is missing"))?
        .try_into()
        .map_err(|_| format!("session property '{name}' is out of range"))
}

fn required_usize(node: &KdlNode, name: &str) -> Result<usize, String> {
    required_u64(node, name).and_then(|value| {
        usize::try_from(value).map_err(|_| format!("session property '{name}' is out of range"))
    })
}

fn required_u8(node: &KdlNode, name: &str) -> Result<u8, String> {
    required_u64(node, name).and_then(|value| {
        u8::try_from(value).map_err(|_| format!("session property '{name}' is out of range"))
    })
}

fn property_u32(node: &KdlNode, name: &str) -> Result<Option<u32>, String> {
    property_integer(node, name)
        .map(|value| {
            value
                .try_into()
                .map_err(|_| format!("session property '{name}' is out of range"))
        })
        .transpose()
}

fn property_bool(node: &KdlNode, name: &str) -> Option<bool> {
    property_value(node, name).and_then(KdlValue::as_bool)
}

fn property_number(node: &KdlNode, name: &str) -> Option<f64> {
    property_value(node, name)
        .and_then(|value| {
            value
                .as_float()
                .or_else(|| value.as_integer().map(|v| v as f64))
        })
        .filter(|value| value.is_finite())
}

fn insert_size(node: &mut KdlNode, size: SizeSpec, proportion: &str, fixed: &str) {
    match size {
        SizeSpec::Proportion(value) => node.insert(proportion, value),
        SizeSpec::Fixed(value) => node.insert(fixed, value),
    };
}

fn required_size(node: &KdlNode, proportion: &str, fixed: &str) -> Result<SizeSpec, String> {
    match (
        property_number(node, proportion),
        property_number(node, fixed),
    ) {
        (Some(value), None) if value > 0.0 => Ok(SizeSpec::Proportion(value)),
        (None, Some(value)) if value > 0.0 && value <= f64::from(i32::MAX) => {
            Ok(SizeSpec::Fixed(value))
        }
        (Some(_), Some(_)) => Err(format!(
            "session properties '{proportion}' and '{fixed}' are mutually exclusive"
        )),
        _ => Err(format!(
            "session requires one valid size property: '{proportion}' or '{fixed}'"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchical_snapshot_round_trips_through_kdl() {
        let snapshot = SessionSnapshot {
            outputs: vec![OutputSnapshot {
                name: Some("DP-1".into()),
                width: Some(1920),
                height: Some(1080),
                workspaces: vec![WorkspaceSnapshot {
                    id: 42,
                    name: Some("work".into()),
                    index: 2,
                    windows: vec![WindowSnapshot {
                        app_id: "org.example.App".into(),
                        column: 3,
                        tile: 1,
                        floating: true,
                        focused: true,
                        width: SizeSpec::Proportion(0.5),
                        height: SizeSpec::Fixed(600.0),
                    }],
                }],
            }],
        };
        let serialized = serialize(&snapshot);
        assert!(serialized.contains("wp=0.5"));
        assert!(serialized.contains("hf=600.0"));
        assert_eq!(parse(&serialized).unwrap(), snapshot);
    }

    #[test]
    fn unsupported_version_is_rejected() {
        assert!(parse("format-version 1").is_err());
    }

    #[test]
    fn invalid_layout_indices_are_rejected() {
        let content = r#"
            format-version 2
            output name="DP-1" {
                workspace id=1 idx=1 {
                    window app="demo" c=-1 t=1 wp=0.5 hp=1.0
                }
            }
        "#;
        assert!(parse(content).is_err());
    }

    #[test]
    fn missing_or_ambiguous_size_modes_are_rejected() {
        let missing = r#"
            format-version 2
            output {
                workspace id=1 idx=1 {
                    window app="demo" c=1 t=1 hp=1.0
                }
            }
        "#;
        let ambiguous = r#"
            format-version 2
            output {
                workspace id=1 idx=1 {
                    window app="demo" c=1 t=1 wp=0.5 wf=800.0 hp=1.0
                }
            }
        "#;
        assert!(parse(missing).is_err());
        assert!(parse(ambiguous).is_err());
    }
}
