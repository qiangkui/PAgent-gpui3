//! Sidebar categories.
//!
//! The left panel groups conversations under collapsible categories. The list
//! lives next to the `chats` and `workflows` folders in the data directory
//! (`<data dir>\sidebar.json`), so a restart keeps both the names and which one
//! is open. Which category a workflow card sits in is stored with that card, in
//! [`crate::workflow_store`].
//!
//! Only the stored shape is here; the window owns the live category type.

use std::collections::HashMap;
use std::path::Path;

/// One category as it is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredCategory {
    /// Stable identifier the conversations point at.
    pub id: String,
    /// Name shown in the sidebar.
    pub name: String,
}

/// The saved sidebar: the categories, which one is open, and how wide the panel
/// was dragged.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredSidebar {
    /// Categories in display order; the first one is the default.
    pub categories: Vec<StoredCategory>,
    /// Id of the open category; empty when they are all closed.
    pub open: String,
    /// Width of the sidebar in logical pixels; `None` when it was never dragged.
    pub width: Option<f32>,
    /// Folder of the conversation that was open; empty when unknown.
    pub active: String,
    /// Display names for workflow cards, keyed by manifest name. A card renamed
    /// in the window keeps its manifest untouched; this map is presentation only.
    pub labels: HashMap<String, String>,
    /// Workflows whose current tab is deliberately blank after “新建运行”.
    /// Kept across restarts so opening the workflow does not resurrect the last
    /// finished run.
    pub cleared: Vec<String>,
}

/// Read the sidebar from a data directory.
pub fn load(dir: &Path) -> Option<StoredSidebar> {
    load_from(&dir.join("sidebar.json"))
}

/// Write the sidebar into a data directory, creating it on first use.
pub fn save(dir: &Path, sidebar: &StoredSidebar) -> Result<(), String> {
    save_to(&dir.join("sidebar.json"), sidebar)
}

/// Read a sidebar from an arbitrary file.
fn load_from(file: &Path) -> Option<StoredSidebar> {
    let text = std::fs::read_to_string(file).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let categories: Vec<StoredCategory> = value
        .get("categories")
        .and_then(|item| item.as_array())
        .map(|list| list.iter().filter_map(read_category).collect())
        .unwrap_or_default();
    if categories.is_empty() {
        return None;
    }
    Some(StoredSidebar {
        categories,
        open: value
            .get("open")
            .and_then(|item| item.as_str())
            .unwrap_or_default()
            .to_string(),
        width: value
            .get("width")
            .and_then(|item| item.as_f64())
            .map(|width| width as f32)
            .filter(|width| width.is_finite() && *width > 0.),
        active: value
            .get("active")
            .and_then(|item| item.as_str())
            .unwrap_or_default()
            .to_string(),
        labels: value
            .get("labels")
            .and_then(|item| item.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(name, label)| {
                        let label = label.as_str()?.trim();
                        if label.is_empty() {
                            None
                        } else {
                            Some((name.clone(), label.to_string()))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        cleared: value
            .get("cleared")
            .and_then(|item| item.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Rebuild one category, or `None` when it carries no name.
fn read_category(value: &serde_json::Value) -> Option<StoredCategory> {
    let name = value.get("name").and_then(|item| item.as_str())?.trim();
    if name.is_empty() {
        return None;
    }
    let id = value
        .get("id")
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    Some(StoredCategory {
        id: if id.trim().is_empty() {
            slug(name)
        } else {
            id.trim().to_string()
        },
        name: name.to_string(),
    })
}

/// Write a sidebar to an arbitrary file, replacing what is there.
fn save_to(file: &Path, sidebar: &StoredSidebar) -> Result<(), String> {
    let items: Vec<serde_json::Value> = sidebar
        .categories
        .iter()
        .map(|category| serde_json::json!({ "id": category.id, "name": category.name }))
        .collect();
    let body = serde_json::json!({
        "categories": items,
        "open": sidebar.open,
        "width": sidebar.width,
        "active": sidebar.active,
        "labels": sidebar.labels,
        "cleared": sidebar.cleared,
    })
    .to_string();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 {}：{error}", parent.display()))?;
    }
    // Atomic, and skipped when the text did not change: the sidebar is written
    // on every switch, so a crash mid-write must not truncate the categories.
    crate::store::write_if_changed(file, &body)
}

/// A stable id derived from a name, for files written without one.
fn slug(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "category".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A throw-away file path.
    fn temp_file(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dsh-flow-ui-sidebar-{}-{tag}.json",
            std::process::id()
        ))
    }

    #[test]
    fn round_trips_categories_and_the_open_one() {
        let file = temp_file("round");
        let sidebar = StoredSidebar {
            categories: vec![
                StoredCategory {
                    id: "c1".to_string(),
                    name: "对话".to_string(),
                },
                StoredCategory {
                    id: "c2".to_string(),
                    name: "工作笔记".to_string(),
                },
            ],
            open: "c2".to_string(),
            width: Some(300.),
            active: "chat-2".to_string(),
            labels: HashMap::from([(
                "maers-units-export".to_string(),
                "使用单位导出".to_string(),
            )]),
            cleared: vec!["maers-units-export".to_string()],
        };
        save_to(&file, &sidebar).unwrap();
        let loaded = load_from(&file);
        let _ = std::fs::remove_file(&file);

        assert_eq!(loaded, Some(sidebar));
    }

    #[test]
    fn tolerates_a_missing_or_broken_file() {
        assert!(load_from(&temp_file("missing")).is_none());
        let file = temp_file("broken");
        std::fs::write(&file, "{not json").unwrap();
        assert!(load_from(&file).is_none());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn ignores_categories_without_a_name() {
        let file = temp_file("sparse");
        std::fs::write(
            &file,
            r#"{"categories":[{"id":"x","name":"  "},{"name":"笔记"}],"open":"x"}"#,
        )
        .unwrap();
        let loaded = load_from(&file).unwrap();
        let _ = std::fs::remove_file(&file);

        assert_eq!(loaded.categories.len(), 1);
        // A file without ids still gets a stable one, so chats can point at it.
        assert_eq!(loaded.categories[0].name, "笔记");
        assert!(!loaded.categories[0].id.is_empty());
    }

    #[test]
    fn ignores_a_nonsense_width() {
        let file = temp_file("width");
        std::fs::write(
            &file,
            r#"{"categories":[{"id":"c1","name":"对话"}],"width":-5}"#,
        )
        .unwrap();
        let loaded = load_from(&file).unwrap();
        let _ = std::fs::remove_file(&file);
        assert_eq!(loaded.width, None);
    }

    #[test]
    fn writes_the_documented_shape() {
        let file = temp_file("shape");
        let sidebar = StoredSidebar {
            categories: vec![StoredCategory {
                id: "c1".to_string(),
                name: "对话".to_string(),
            }],
            open: "c1".to_string(),
            ..StoredSidebar::default()
        };
        save_to(&file, &sidebar).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let _ = std::fs::remove_file(&file);

        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["categories"][0]["id"], "c1");
        assert_eq!(value["categories"][0]["name"], "对话");
        assert_eq!(value["open"], "c1");
        assert_eq!(value["labels"], serde_json::json!({}));
        assert_eq!(value["cleared"], serde_json::json!([]));
        // A sidebar that was never dragged writes no width at all.
        assert!(value["width"].is_null());
        // Workflow cards are not filed here any more; their folders are.
        assert!(value.get("workflows").is_none());
    }
}
