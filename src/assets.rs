//! Icons embedded in the binary.
//!
//! The window draws its tool buttons with monochrome SVGs tinted by the theme,
//! so they keep a consistent shape instead of depending on which glyph the system
//! font happens to have. The files live in `assets/icons` and are compiled in, so
//! the binary has no runtime asset folder.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Settings: two sliders.
const SETTINGS: &[u8] = include_bytes!("../assets/icons/settings.svg");
/// Plugins: a puzzle piece.
const PLUGINS: &[u8] = include_bytes!("../assets/icons/plugins.svg");
/// Theme: clothing, the "change the outfit" metaphor.
const THEME: &[u8] = include_bytes!("../assets/icons/theme.svg");
/// The product mark: a robot head, drawn at the left of the title bar.
const APP: &[u8] = include_bytes!("../assets/icons/app.svg");
/// Stats: three bars.
const STATS: &[u8] = include_bytes!("../assets/icons/stats.svg");
/// MCP: two linked nodes.
const MCP: &[u8] = include_bytes!("../assets/icons/mcp.svg");
/// Skill: a lightbulb.
const SKILL: &[u8] = include_bytes!("../assets/icons/skill.svg");
/// Copy: two sheets, beside a message.
const COPY: &[u8] = include_bytes!("../assets/icons/copy.svg");
/// Save: an arrow into a tray, beside the copy button.
const SAVE: &[u8] = include_bytes!("../assets/icons/save.svg");
/// Regenerate: a circular arrow, under an answer.
const REGENERATE: &[u8] = include_bytes!("../assets/icons/regenerate2.svg");
/// Submit: the arrow in the round send button at the composer's end.
const SUBMIT: &[u8] = include_bytes!("../assets/icons/submit.svg");
/// Rename: a pencil, beside a conversation title.
const RENAME: &[u8] = include_bytes!("../assets/icons/rename.svg");
/// Plus: starts a new conversation, in the sidebar's list.
const PLUS: &[u8] = include_bytes!("../assets/icons/plus.svg");
/// Search: the magnifier beside the conversation filter.
const SEARCH: &[u8] = include_bytes!("../assets/icons/search.svg");
/// Folder: a folded category in the 工作区 tree.
const FOLDER: &[u8] = include_bytes!("../assets/icons/folder.svg");
/// Open folder: the unfolded category in the 工作区 tree.
const FOLDER_OPEN: &[u8] = include_bytes!("../assets/icons/folder-open.svg");
/// Ellipsis: the menu button at the end of a folder row.
const ELLIPSIS: &[u8] = include_bytes!("../assets/icons/ellipsis.svg");
/// Tasks: a terminal window, beside the conversation header's find button.
const TASKS: &[u8] = include_bytes!("../assets/icons/tasks.svg");
/// Layers: three stacked sheets, beside the composer capsule's context figure.
const LAYERS: &[u8] = include_bytes!("../assets/icons/layers.svg");
/// Text file: the 文本 option in the composer's attach menu.
const FILE_TEXT: &[u8] = include_bytes!("../assets/icons/file-text.svg");
/// Image file: the 图片 option in the composer's attach menu.
const IMAGE: &[u8] = include_bytes!("../assets/icons/image.svg");
/// PDF file: the PDF option in the composer's attach menu.
const PDF: &[u8] = include_bytes!("../assets/icons/pdf.svg");
/// Compress: the active-compaction command in the composer's attach menu.
const COMPRESS: &[u8] = include_bytes!("../assets/icons/compress.svg");
/// Plus (thin): the round add button beside the composer's model chip.
const PLUS_THIN: &[u8] = include_bytes!("../assets/icons/plus-thin.svg");
/// Permission 仅可查看: the shield-with-check glyph dsh uses for `read-only`.
const PERMISSION_READ_ONLY: &[u8] = include_bytes!("../assets/icons/permission-read-only.svg");
/// Permission 工作区内修改: dsh's shield-with-pencil glyph.
const PERMISSION_WORKSPACE: &[u8] = include_bytes!("../assets/icons/permission-workspace.svg");
/// Permission 完全权限: dsh's shield-with-bar glyph.
const PERMISSION_FULL: &[u8] = include_bytes!("../assets/icons/permission-full.svg");

/// The icons the window can draw.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(match path {
            "icons/settings.svg" => Some(Cow::Borrowed(SETTINGS)),
            "icons/plugins.svg" => Some(Cow::Borrowed(PLUGINS)),
            "icons/theme.svg" => Some(Cow::Borrowed(THEME)),
            "icons/app.svg" => Some(Cow::Borrowed(APP)),
            "icons/stats.svg" => Some(Cow::Borrowed(STATS)),
            "icons/mcp.svg" => Some(Cow::Borrowed(MCP)),
            "icons/skill.svg" => Some(Cow::Borrowed(SKILL)),
            "icons/copy.svg" => Some(Cow::Borrowed(COPY)),
            "icons/save.svg" => Some(Cow::Borrowed(SAVE)),
            "icons/regenerate2.svg" => Some(Cow::Borrowed(REGENERATE)),
            "icons/submit.svg" => Some(Cow::Borrowed(SUBMIT)),
            "icons/rename.svg" => Some(Cow::Borrowed(RENAME)),
            "icons/plus.svg" => Some(Cow::Borrowed(PLUS)),
            "icons/search.svg" => Some(Cow::Borrowed(SEARCH)),
            "icons/folder.svg" => Some(Cow::Borrowed(FOLDER)),
            "icons/folder-open.svg" => Some(Cow::Borrowed(FOLDER_OPEN)),
            "icons/ellipsis.svg" => Some(Cow::Borrowed(ELLIPSIS)),
            "icons/tasks.svg" => Some(Cow::Borrowed(TASKS)),
            "icons/layers.svg" => Some(Cow::Borrowed(LAYERS)),
            "icons/file-text.svg" => Some(Cow::Borrowed(FILE_TEXT)),
            "icons/image.svg" => Some(Cow::Borrowed(IMAGE)),
            "icons/pdf.svg" => Some(Cow::Borrowed(PDF)),
            "icons/compress.svg" => Some(Cow::Borrowed(COMPRESS)),
            "icons/plus-thin.svg" => Some(Cow::Borrowed(PLUS_THIN)),
            "icons/permission-read-only.svg" => Some(Cow::Borrowed(PERMISSION_READ_ONLY)),
            "icons/permission-workspace.svg" => Some(Cow::Borrowed(PERMISSION_WORKSPACE)),
            "icons/permission-full.svg" => Some(Cow::Borrowed(PERMISSION_FULL)),
            _ => None,
        })
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_embedded() {
        let assets = Assets;
        for path in [
            "icons/settings.svg",
            "icons/plugins.svg",
            "icons/theme.svg",
            "icons/app.svg",
            "icons/stats.svg",
            "icons/mcp.svg",
            "icons/skill.svg",
            "icons/copy.svg",
            "icons/save.svg",
            "icons/regenerate2.svg",
            "icons/submit.svg",
            "icons/rename.svg",
            "icons/plus.svg",
            "icons/search.svg",
            "icons/folder.svg",
            "icons/folder-open.svg",
            "icons/ellipsis.svg",
            "icons/tasks.svg",
            "icons/layers.svg",
            "icons/file-text.svg",
            "icons/image.svg",
            "icons/pdf.svg",
            "icons/compress.svg",
            "icons/plus-thin.svg",
            "icons/permission-read-only.svg",
            "icons/permission-workspace.svg",
            "icons/permission-full.svg",
        ] {
            let bytes = assets.load(path).expect("load").expect("icon present");
            // Icons drawn by hand start with `<svg`; the ones taken from an icon
            // set carry an XML prolog and a doctype in front of it.
            let svg = String::from_utf8_lossy(&bytes);
            assert!(svg.contains("<svg"), "{path} is not an svg");
            assert!(svg.contains("</svg>"), "{path} is not a closed svg");
        }
        assert!(assets.load("icons/missing.svg").expect("load").is_none());
    }
}
