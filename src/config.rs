use kdl::{KdlDocument, KdlNode, KdlValue};
use std::path::PathBuf;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppConfig {
    pub show_workspace_names: bool,
    pub session_enabled: bool,
    pub fixed_snapshot_enabled: bool,
    pub fixed_snapshot_enabled_at: Option<String>,
    pub pinned: Vec<String>,
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("cosmic-ext-niri-windows").join("config.kdl"))
}

pub fn load() -> Result<AppConfig, String> {
    let Some(path) = config_path() else {
        return Ok(AppConfig::default());
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => parse(&content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => migrate_legacy(),
        Err(error) => Err(format!("failed to read config {:?}: {error}", path)),
    }
}

fn migrate_legacy() -> Result<AppConfig, String> {
    let mut config = AppConfig::default();
    if let Some(path) = legacy_pinned_path()
        && let Ok(content) = std::fs::read_to_string(path)
        && let Ok(pinned) = serde_json::from_str::<Vec<String>>(&content)
    {
        config.pinned = pinned
            .into_iter()
            .map(|id| crate::applications::resolve_canonical_desktop_id(&id))
            .collect();
    }
    if let Some(path) = crate::session::session_path() {
        config.session_enabled = crate::session::legacy_enabled(&path).unwrap_or(false);
    }
    Ok(config)
}

pub async fn save(config: &AppConfig) -> Result<(), String> {
    let path = config_path().ok_or_else(|| "config path is unavailable".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "config path has no parent directory".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("failed to create config directory: {error}"))?;
    let temporary = path.with_extension("kdl.tmp");
    tokio::fs::write(&temporary, serialize(config))
        .await
        .map_err(|error| format!("failed to write temporary config: {error}"))?;
    tokio::fs::rename(&temporary, &path)
        .await
        .map_err(|error| format!("failed to commit config file: {error}"))
}

pub fn serialize(config: &AppConfig) -> String {
    let mut document = KdlDocument::new();
    let mut node = KdlNode::new("session-enabled");
    node.push(config.session_enabled);
    document.nodes_mut().push(node);
    let mut node = KdlNode::new("show-workspace-names");
    node.push(config.show_workspace_names);
    document.nodes_mut().push(node);
    let mut node = KdlNode::new("fixed-snapshot-enabled");
    node.push(config.fixed_snapshot_enabled);
    document.nodes_mut().push(node);
    if let Some(enabled_at) = &config.fixed_snapshot_enabled_at {
        let mut node = KdlNode::new("fixed-snapshot-enabled-at");
        node.push(enabled_at.clone());
        document.nodes_mut().push(node);
    }

    let mut pinned = KdlNode::new("pinned");
    let pinned_children = pinned.ensure_children();
    for app_id in &config.pinned {
        let mut pinned_node = KdlNode::new(app_id.as_str());
        pinned_node.clear();
        pinned_children.nodes_mut().push(pinned_node);
    }
    document.nodes_mut().push(pinned);
    document.to_string()
}

pub fn parse(content: &str) -> Result<AppConfig, String> {
    if content.len() > MAX_CONFIG_BYTES as usize {
        return Err("config file exceeds the maximum supported size".to_string());
    }
    let document: KdlDocument = content
        .parse()
        .map_err(|error| format!("invalid config KDL: {error}"))?;
    let mut config = AppConfig::default();
    for node in document.nodes() {
        match node.name().value() {
            "session-enabled" => {
                config.session_enabled = node_value(node)
                    .and_then(KdlValue::as_bool)
                    .ok_or_else(|| "session-enabled must be boolean".to_string())?;
            }
            "show-workspace-names" => {
                config.show_workspace_names = node_value(node)
                    .and_then(KdlValue::as_bool)
                    .ok_or_else(|| "show-workspace-names must be boolean".to_string())?;
            }
            "fixed-snapshot-enabled" => {
                config.fixed_snapshot_enabled = node_value(node)
                    .and_then(KdlValue::as_bool)
                    .ok_or_else(|| "fixed-snapshot-enabled must be boolean".to_string())?;
            }
            "fixed-snapshot-enabled-at" => {
                config.fixed_snapshot_enabled_at = node_value(node)
                    .and_then(KdlValue::as_string)
                    .map(ToOwned::to_owned);
            }
            "pinned" => {
                if let Some(pinned_children) = node.children() {
                    for pinned_node in pinned_children.nodes() {
                        config
                            .pinned
                            .push(crate::applications::resolve_canonical_desktop_id(
                                pinned_node.name().value(),
                            ));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(config)
}

fn node_value(node: &KdlNode) -> Option<&KdlValue> {
    node.get(0)
}

fn legacy_pinned_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("cosmic-ext-niri-windows").join("pinned.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips() {
        let config = AppConfig {
            show_workspace_names: true,
            session_enabled: true,
            fixed_snapshot_enabled: true,
            fixed_snapshot_enabled_at: Some("2026-08-11T12:00:00Z".into()),
            pinned: vec!["org.example.App".into()],
        };
        assert_eq!(parse(&serialize(&config)).unwrap(), config);
    }
}
