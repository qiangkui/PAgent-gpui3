//! Installed skills: folders of prompt text the assistant reads on every
//! question.
//!
//! A skill is a folder under the program's `skills` folder. Its `skill.json`
//! names and describes it; the instructions come from that file's
//! `instructions` field, or from `SKILL.md` next to it. Only folders switched on
//! in `config.json` reach the prompt.

use std::path::{Path, PathBuf};

use crate::config::{AiSettings, is_plugin_id};

/// Most skill folders the page will list, and the most ids the config keeps.
pub const MAX_SKILLS: usize = 64;

/// Longest instructions one skill may contribute, in characters.
///
/// Skills ride on every question; an accidentally pasted book would crowd out
/// the conversation, so the read is capped with a visible note.
pub const MAX_INSTRUCTIONS: usize = 20_000;

/// Longest combined skill text sent in one prompt, in characters.
pub const MAX_PROMPT: usize = 20_000;

/// One skill folder, whether or not it can be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledSkill {
    /// Folder name, which is also the id stored in `config.json`.
    pub id: String,
    /// Display name from `skill.json`, or the id.
    pub name: String,
    /// One-line description from `skill.json`.
    pub description: String,
    /// Prompt text injected when the skill is switched on.
    pub instructions: String,
    /// Why the folder cannot be used; empty when it looks usable.
    pub problem: String,
}

/// The folder holding installed skills.
pub fn skill_dir() -> Option<PathBuf> {
    AiSettings::skill_dir()
}

/// List every skill folder, sorted by name.
pub fn installed_skills() -> Vec<InstalledSkill> {
    skill_dir()
        .map(|dir| scanned_skills(&dir))
        .unwrap_or_default()
}

/// The same scan against an arbitrary folder (the tests use this one).
pub fn scanned_skills(dir: &Path) -> Vec<InstalledSkill> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if !is_plugin_id(&id) {
            continue;
        }
        found.push(read_skill(&id, &path));
    }
    found.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    found.truncate(MAX_SKILLS);
    found
}

/// Read one folder's manifest and instructions.
fn read_skill(id: &str, folder: &Path) -> InstalledSkill {
    let mut skill = InstalledSkill {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        instructions: String::new(),
        problem: String::new(),
    };
    let manifest = folder.join("skill.json");
    match std::fs::read_to_string(&manifest) {
        Ok(text) => {
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            match serde_json::from_str::<serde_json::Value>(text) {
                Ok(value) => {
                    if let Some(name) = text_field(&value, "name") {
                        skill.name = name;
                    }
                    skill.description = text_field(&value, "description").unwrap_or_default();
                    skill.instructions = text_field(&value, "instructions").unwrap_or_default();
                }
                Err(error) => skill.problem = format!("skill.json 不是合法 JSON：{error}"),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            skill.problem = "缺少 skill.json".to_string();
        }
        Err(error) => skill.problem = format!("无法读取 skill.json：{error}"),
    }
    if skill.instructions.trim().is_empty() {
        for name in ["SKILL.md", "skill.md"] {
            if let Ok(text) = std::fs::read_to_string(folder.join(name)) {
                let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                skill.instructions = text.to_string();
                if !skill.instructions.trim().is_empty() {
                    break;
                }
            }
        }
    }
    if skill.instructions.trim().is_empty() && skill.problem.is_empty() {
        skill.problem = "缺少 instructions 或 SKILL.md".to_string();
    }
    if skill.instructions.chars().count() > MAX_INSTRUCTIONS {
        skill.instructions = skill
            .instructions
            .chars()
            .take(MAX_INSTRUCTIONS)
            .collect::<String>();
        skill.instructions.push_str("\n\n（技能内容过长，已截断）");
    }
    skill
}

/// Read a trimmed string field, treating blank as absent.
fn text_field(value: &serde_json::Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Every switched-on skill that can be used, in name order.
pub fn enabled_skills(settings: &AiSettings) -> Vec<InstalledSkill> {
    installed_skills()
        .into_iter()
        .filter(|skill| skill.problem.is_empty() && settings.skill_enabled(&skill.id))
        .collect()
}

/// The system-prompt block for the enabled skills in `allowed`.
///
/// A mode narrows what a request carries; the global Skill page still decides
/// whether a skill may be used at all. An empty `allowed` means no skill.
pub fn prompt_text_for(settings: &AiSettings, allowed: &[String]) -> String {
    if allowed.is_empty() {
        return String::new();
    }
    let allowed: std::collections::HashSet<&str> =
        allowed.iter().map(String::as_str).collect();
    let skills: Vec<InstalledSkill> = enabled_skills(settings)
        .into_iter()
        .filter(|skill| allowed.contains(skill.id.as_str()))
        .collect();
    join_instructions(&skills)
}

#[allow(dead_code)]
/// The system-prompt block for every enabled skill, or an empty string.
pub fn prompt_text(settings: &AiSettings) -> String {
    join_instructions(&enabled_skills(settings))
}

/// Join skills into one block, stopping before the total budget is spent.
pub fn join_instructions(skills: &[InstalledSkill]) -> String {
    let mut text = String::new();
    for skill in skills {
        if skill.instructions.trim().is_empty() {
            continue;
        }
        let piece = format!("## 技能：{}\n\n{}", skill.name, skill.instructions.trim());
        let used = text.chars().count();
        if used > 0 && used + piece.chars().count() > MAX_PROMPT {
            text.push_str("\n\n（其余技能因长度上限未注入）");
            break;
        }
        if used > 0 {
            text.push_str("\n\n");
        }
        text.push_str(&piece);
    }
    text
}

/// Delete one skill folder, if it exists.
pub fn delete_skill(id: &str) -> Result<(), String> {
    let Some(dir) = skill_dir() else {
        return Err("找不到技能目录".to_string());
    };
    delete_skill_in(&dir, id)
}

/// The same deletion against an arbitrary folder.
pub fn delete_skill_in(dir: &Path, id: &str) -> Result<(), String> {
    if !is_plugin_id(id) {
        return Err(format!("技能 id「{id}」不合法"));
    }
    let folder = dir.join(id);
    if !folder.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&folder)
        .map_err(|error| format!("无法删除 {}：{error}", folder.display()))
}

/// Copy a skill folder in from anywhere on disk, then return its id.
///
/// The folder name is the id, and the id is what `config.json` keeps, so the
/// import checks it the way the scanner does: an unusable name, a manifest the
/// page would report as broken, and an id that is already installed are all
/// refused before anything is copied.
pub fn import_skill(source: &Path) -> Result<String, String> {
    let Some(dir) = skill_dir() else {
        return Err("找不到技能目录".to_string());
    };
    import_skill_into(source, &dir)
}

/// The same import against an arbitrary folder (the tests use this one).
pub fn import_skill_into(source: &Path, dir: &Path) -> Result<String, String> {
    if !source.is_dir() {
        return Err("请选择一个技能文件夹（里面放 skill.json）。".to_string());
    }
    let Some(id) = source
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
    else {
        return Err(format!("读不出 {} 的文件夹名", source.display()));
    };
    if !is_plugin_id(&id) {
        return Err(format!(
            "文件夹名「{id}」不能当技能 id：只能用字母、数字、-、_、.，且不能以 . 开头或结尾"
        ));
    }
    let skill = read_skill(&id, source);
    if !skill.problem.is_empty() {
        return Err(format!("这个文件夹不是可用的技能：{}", skill.problem));
    }
    if scanned_skills(dir).len() >= MAX_SKILLS {
        return Err(format!("技能已经有 {MAX_SKILLS} 个，先删掉一个再添加"));
    }
    let target = dir.join(&id);
    if target.exists() {
        return Err(format!("技能「{id}」已经存在，先删除它再添加"));
    }
    crate::folder::copy_dir(source, &target)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dsh-flow-ui-skill-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_inline_instructions_and_the_markdown_fallback() {
        let dir = temp_dir("reads");
        let inline = dir.join("inline");
        std::fs::create_dir_all(&inline).unwrap();
        std::fs::write(
            inline.join("skill.json"),
            r#"{"name":"内联","description":"写在 JSON 里","instructions":"按这个做。"}"#,
        )
        .unwrap();
        let markdown = dir.join("markdown");
        std::fs::create_dir_all(&markdown).unwrap();
        std::fs::write(
            markdown.join("skill.json"),
            r#"{"name":"Markdown","description":"写在 SKILL.md 里"}"#,
        )
        .unwrap();
        std::fs::write(markdown.join("SKILL.md"), "# 标题\n\n先看 README。").unwrap();

        let found = scanned_skills(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Markdown");
        assert_eq!(found[0].instructions, "# 标题\n\n先看 README。");
        assert!(found[0].problem.is_empty());
        assert_eq!(found[1].name, "内联");
        assert_eq!(found[1].instructions, "按这个做。");
    }

    #[test]
    fn explains_a_broken_manifest_and_missing_instructions() {
        let dir = temp_dir("broken");
        let broken = dir.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("skill.json"), "{not json").unwrap();
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join("skill.json"), r#"{"name":"空"}"#).unwrap();
        let missing = dir.join("missing");
        std::fs::create_dir_all(&missing).unwrap();

        let found = scanned_skills(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found.len(), 3);
        let problem = |id: &str| {
            found
                .iter()
                .find(|skill| skill.id == id)
                .map(|skill| skill.problem.clone())
                .unwrap_or_default()
        };
        assert!(problem("broken").contains("不是合法 JSON"));
        assert!(problem("empty").contains("instructions"));
        assert!(problem("missing").contains("skill.json"));
    }

    #[test]
    fn deletes_only_the_named_skill_folder() {
        let dir = temp_dir("delete");
        for id in ["gone", "kept"] {
            let folder = dir.join(id);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("skill.json"), r#"{"instructions":"x"}"#).unwrap();
        }
        delete_skill_in(&dir, "gone").unwrap();
        assert!(!dir.join("gone").exists());
        assert!(dir.join("kept").exists());
        assert!(delete_skill_in(&dir, "../evil").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn imports_a_valid_skill_folder() {
        let dir = temp_dir("import");
        let source = dir.join("source").join("polish");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("skill.json"),
            r#"{"name":"润色","description":"改文案"}"#,
        )
        .unwrap();
        std::fs::write(source.join("SKILL.md"), "先通读，再改。").unwrap();
        let target = dir.join("skills");
        std::fs::create_dir_all(&target).unwrap();

        let id = import_skill_into(&source, &target).unwrap();
        assert_eq!(id, "polish");
        assert_eq!(
            std::fs::read_to_string(target.join("polish").join("SKILL.md")).unwrap(),
            "先通读，再改。"
        );
        let found = scanned_skills(&target);
        assert_eq!(found.len(), 1);
        assert!(found[0].problem.is_empty());
        // The original is left alone, and the same id cannot land twice.
        assert!(source.join("SKILL.md").is_file());
        assert!(import_skill_into(&source, &target).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_a_folder_that_is_not_a_skill() {
        let dir = temp_dir("import-broken");
        let target = dir.join("skills");
        std::fs::create_dir_all(&target).unwrap();

        let missing = dir.join("missing");
        std::fs::create_dir_all(&missing).unwrap();
        assert!(
            import_skill_into(&missing, &target)
                .unwrap_err()
                .contains("skill.json")
        );

        let spaced = dir.join("有 空格");
        std::fs::create_dir_all(&spaced).unwrap();
        std::fs::write(spaced.join("skill.json"), r#"{"instructions":"x"}"#).unwrap();
        assert!(
            import_skill_into(&spaced, &target)
                .unwrap_err()
                .contains("技能 id")
        );

        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join("skill.json"), r#"{"name":"空"}"#).unwrap();
        assert!(
            import_skill_into(&empty, &target)
                .unwrap_err()
                .contains("instructions")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn joins_skills_until_the_prompt_budget_is_spent() {
        let skill = |id: &str, text: &str| InstalledSkill {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            instructions: text.to_string(),
            problem: String::new(),
        };
        let short = join_instructions(&[skill("a", "先看 README。"), skill("b", "再跑测试。")]);
        assert!(short.contains("## 技能：a"));
        assert!(short.contains("再跑测试。"));

        let long = "x".repeat(MAX_PROMPT);
        let capped = join_instructions(&[skill("a", &long), skill("b", "加不进去")]);
        assert!(capped.contains("长度上限"));
        assert!(!capped.contains("加不进去"));
    }
}
