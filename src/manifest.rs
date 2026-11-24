//! A small reader for the manifest fields the window shows before a run.
//!
//! The engine owns the authoritative schema: it validates every key, every
//! template and every path. This module only extracts what the plan pane needs
//! (identity, inputs, vars, step order) so the view can render before anything
//! executes. Anything it cannot read is skipped, never guessed, and the engine
//! remains the only source of truth for run results.

/// The identity and shape of one manifest, as far as the UI cares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestSpec {
    /// `name:` from the manifest.
    pub name: Option<String>,
    /// `description:` from the manifest.
    pub description: Option<String>,
    /// `version:` from the manifest.
    pub version: Option<String>,
    /// Declared inputs, in manifest order.
    pub inputs: Vec<InputSpec>,
    /// Declared vars, in manifest order.
    pub vars: Vec<(String, String)>,
    /// Declared steps, in execution order.
    pub steps: Vec<StepSpec>,
}

/// One entry under `inputs:`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputSpec {
    /// Input name, used for `--input name=value`.
    pub name: String,
    /// Help text shown next to the field.
    pub description: String,
    /// True when the manifest marks the input required.
    pub required: bool,
    /// Default value the engine applies when the input is omitted.
    pub default: Option<String>,
}

impl InputSpec {
    /// The value the field starts with: the declared default, or empty.
    pub fn initial_value(&self) -> String {
        self.default.clone().unwrap_or_default()
    }

    /// Text used when the field is empty.
    pub fn placeholder(&self) -> String {
        if self.required {
            "required".to_string()
        } else {
            "default".to_string()
        }
    }
}

/// Which executor a step declares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StepKind {
    /// `run:` — spawns a real program.
    Run,
    /// `set:` — writes literal values.
    Set,
    /// `map:` — fans out over a collection.
    Map,
    /// `ai:` — an explicit model call.
    Ai,
    /// A step whose executor could not be read.
    #[default]
    Unknown,
}

impl StepKind {
    /// Short label for the plan list.
    pub fn label(self) -> &'static str {
        match self {
            StepKind::Run => "run",
            StepKind::Set => "set",
            StepKind::Map => "map",
            StepKind::Ai => "ai",
            StepKind::Unknown => "?",
        }
    }
}

/// One entry under `steps:`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepSpec {
    /// `id:` of the step.
    pub id: String,
    /// `desc:` of the step.
    pub description: String,
    /// Declared executor.
    pub kind: StepKind,
    /// Human-readable summary: command, keys, or prompt.
    pub summary: String,
    /// `when:` guard, when present.
    pub when: Option<String>,
    /// `save:` binding, when present.
    pub save: Option<String>,
    /// Extra flags worth showing: `skip`, `allowFailure`, `retries`, `timeout`.
    pub notes: Vec<String>,
}

/// Where the reader currently is in the manifest.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Inputs,
    Vars,
    Steps,
    Other,
}

/// One indented, non-empty line of a manifest.
struct Line<'a> {
    indent: usize,
    text: &'a str,
}

/// Read a manifest from YAML or JSON text.
pub fn parse(text: &str) -> ManifestSpec {
    let trimmed = text.trim_start_matches('\u{feff}');
    if trimmed.trim_start().starts_with('{')
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
    {
        return parse_json(&value);
    }

    let lines = split_lines(trimmed);
    let mut spec = ManifestSpec::default();
    let mut section = Section::Other;
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        if line.indent == 0 {
            // `description: |` and friends: the text is the indented block below,
            // not the marker itself.
            if let Some((key, value)) = split_field(line.text)
                && matches!(key, "name" | "description" | "version")
                && block_marker(value)
            {
                let (text, next) = read_block_scalar(&lines, index);
                match key {
                    "name" => spec.name = Some(text),
                    "version" => spec.version = Some(text),
                    _ => spec.description = Some(text),
                }
                index = next;
                continue;
            }
            match split_field(line.text) {
                Some((key, value)) => match key {
                    "name" => spec.name = Some(scalar(value)),
                    "description" => spec.description = Some(scalar(value)),
                    "version" => spec.version = Some(scalar(value)),
                    "inputs" => section = Section::Inputs,
                    "vars" => section = Section::Vars,
                    "steps" => section = Section::Steps,
                    _ => section = Section::Other,
                },
                None => section = Section::Other,
            }
            index += 1;
            continue;
        }
        index = match section {
            Section::Inputs => read_input(&lines, index, &mut spec),
            Section::Vars => {
                read_var(&lines, index, &mut spec);
                index + 1
            }
            Section::Steps => read_step(&lines, index, &mut spec),
            Section::Other => index + 1,
        };
    }
    spec
}

/// Read an already-parsed JSON manifest.
pub fn parse_json(value: &serde_json::Value) -> ManifestSpec {
    let mut spec = ManifestSpec {
        name: string_at(value, "name"),
        description: string_at(value, "description"),
        version: string_at(value, "version"),
        ..Default::default()
    };

    if let Some(inputs) = value.get("inputs").and_then(|entry| entry.as_object()) {
        for (name, definition) in inputs {
            let mut input = InputSpec {
                name: name.clone(),
                ..Default::default()
            };
            if let Some(object) = definition.as_object() {
                input.description = string_at(definition, "description").unwrap_or_default();
                input.required = object
                    .get("required")
                    .and_then(|flag| flag.as_bool())
                    .unwrap_or(false);
                input.default = object.get("default").map(json_scalar);
            } else if !definition.is_null() {
                input.default = Some(json_scalar(definition));
            }
            spec.inputs.push(input);
        }
    }

    if let Some(vars) = value.get("vars").and_then(|entry| entry.as_object()) {
        for (name, declared) in vars {
            spec.vars.push((name.clone(), json_scalar(declared)));
        }
    }

    if let Some(steps) = value.get("steps").and_then(|entry| entry.as_array()) {
        for (order, step) in steps.iter().enumerate() {
            let mut entry = StepSpec {
                id: string_at(step, "id").unwrap_or_else(|| format!("#{}", order + 1)),
                description: string_at(step, "desc")
                    .or_else(|| string_at(step, "description"))
                    .unwrap_or_default(),
                when: string_at(step, "when"),
                save: string_at(step, "save"),
                ..Default::default()
            };
            if let Some(run) = step.get("run") {
                entry.kind = StepKind::Run;
                entry.summary = json_command(run);
            } else if let Some(set) = step.get("set") {
                entry.kind = StepKind::Set;
                entry.summary = json_keys(set);
            } else if let Some(map) = step.get("map") {
                entry.kind = StepKind::Map;
                entry.summary = json_map_summary(map);
            } else if let Some(ai) = step.get("ai") {
                entry.kind = StepKind::Ai;
                entry.summary = json_ai_summary(ai);
            }
            if step.get("skip").is_some() {
                entry.notes.push("skip".to_string());
            }
            if step.get("allowFailure").and_then(|flag| flag.as_bool()) == Some(true) {
                entry.notes.push("allowFailure".to_string());
            }
            if let Some(retries) = step.get("retries") {
                entry
                    .notes
                    .push(format!("retries {}", json_scalar(retries)));
            }
            spec.steps.push(entry);
        }
    }

    spec
}

/// Collect the meaningful lines of a YAML document with their indentation.
fn split_lines(text: &str) -> Vec<Line<'_>> {
    text.lines()
        .filter_map(|raw| {
            let raw = raw.trim_end();
            let content = raw.trim_start();
            if content.is_empty() || content.starts_with('#') || content.starts_with("---") {
                return None;
            }
            Some(Line {
                indent: raw.len() - content.len(),
                text: content,
            })
        })
        .collect()
}

/// Read one `inputs:` entry, inline or block form, and return the next index.
fn read_input(lines: &[Line<'_>], index: usize, spec: &mut ManifestSpec) -> usize {
    let line = &lines[index];
    let Some((name, value)) = split_field(line.text) else {
        return index + 1;
    };
    let mut input = InputSpec {
        name: scalar(name),
        ..Default::default()
    };

    if let Some(inner) = flow_body(value) {
        for (key, value) in split_flow_pairs(inner) {
            match key.as_str() {
                "description" => input.description = value,
                "required" => input.required = is_truthy(&value),
                "default" => input.default = Some(value),
                _ => {}
            }
        }
        spec.inputs.push(input);
        return index + 1;
    }

    if !value.trim().is_empty() {
        input.default = Some(scalar(value));
        spec.inputs.push(input);
        return index + 1;
    }

    let (block, next) = read_block(lines, index);
    for (key, value) in block {
        match key.as_str() {
            "description" => input.description = value,
            "required" => input.required = is_truthy(&value),
            "default" => input.default = Some(value),
            _ => {}
        }
    }
    spec.inputs.push(input);
    next
}

/// Read one `vars:` entry.
fn read_var(lines: &[Line<'_>], index: usize, spec: &mut ManifestSpec) {
    let Ok((key, value)) = split_field(lines[index].text).ok_or(()) else {
        return;
    };
    if value.trim().is_empty() {
        return;
    }
    spec.vars.push((scalar(key), scalar(value)));
}

/// Read one `steps:` entry, including its nested executor block.
fn read_step(lines: &[Line<'_>], index: usize, spec: &mut ManifestSpec) -> usize {
    let line = &lines[index];
    let base = line.indent;
    let body = line.text.strip_prefix("- ").unwrap_or(line.text).trim();
    let mut step = StepSpec::default();

    if let Some(inner) = flow_body(body) {
        for (key, value) in split_flow_pairs(inner) {
            apply_step_field(&mut step, &key, &value);
        }
        finish_step(spec, step);
        return index + 1;
    }
    if let Some((key, value)) = split_field(body) {
        apply_step_field(&mut step, key, value);
    }

    let mut next = index + 1;
    while next < lines.len() && lines[next].indent > base && !lines[next].text.starts_with("- ") {
        let Some((key, value)) = split_field(lines[next].text) else {
            next += 1;
            continue;
        };
        if matches!(key, "run" | "set" | "map" | "ai")
            && (value.trim().is_empty() || block_marker(value))
        {
            let (raw, after) = read_raw_block(lines, next);
            apply_step_block(&mut step, key, &raw);
            next = after;
            continue;
        }
        apply_step_field(&mut step, key, value);
        next += 1;
    }

    finish_step(spec, step);
    next
}

/// Store a step, giving it a readable placeholder when `id:` was missing.
fn finish_step(spec: &mut ManifestSpec, mut step: StepSpec) {
    if step.id.is_empty() {
        step.id = format!("#{}", spec.steps.len() + 1);
    }
    spec.steps.push(step);
}

/// Read the first-level children of a line as key/value pairs.
fn read_block(lines: &[Line<'_>], parent: usize) -> (Vec<(String, String)>, usize) {
    let base = lines[parent].indent;
    let mut pairs = Vec::new();
    let mut next = parent + 1;
    while next < lines.len() && lines[next].indent > base {
        let Some((key, value)) = split_field(lines[next].text) else {
            next += 1;
            continue;
        };
        if block_marker(value) {
            let (text, after) = read_block_scalar(lines, next);
            pairs.push((scalar(key), text));
            next = after;
            continue;
        }
        pairs.push((scalar(key), scalar(value)));
        next += 1;
    }
    (pairs, next)
}

/// Read the children of a line verbatim, keeping sequence dashes.
fn read_raw_block(lines: &[Line<'_>], parent: usize) -> (Vec<String>, usize) {
    let base = lines[parent].indent;
    let mut raw = Vec::new();
    let mut next = parent + 1;
    while next < lines.len() && lines[next].indent > base {
        raw.push(lines[next].text.to_string());
        next += 1;
    }
    (raw, next)
}

/// Apply one `key: value` pair from a step.
fn apply_step_field(step: &mut StepSpec, key: &str, value: &str) {
    match key {
        "id" => step.id = scalar(value),
        "desc" | "description" => step.description = scalar(value),
        "when" => step.when = Some(scalar(value)),
        "save" => step.save = Some(scalar(value)),
        "skip" => {
            let reason = scalar(value);
            step.notes.push(if reason.is_empty() {
                "skip".to_string()
            } else {
                format!("skip {reason}")
            });
        }
        "allowFailure" => {
            if is_truthy(value) {
                step.notes.push("allowFailure".to_string());
            }
        }
        "retries" => step.notes.push(format!("retries {}", scalar(value))),
        "retryDelayMs" => step.notes.push(format!("retryDelay {} ms", scalar(value))),
        "run" => {
            note_kind(step, StepKind::Run);
            let command = command_from(value);
            set_summary(step, command);
        }
        "set" => {
            note_kind(step, StepKind::Set);
            let keys = flow_keys(value);
            set_summary(step, keys);
        }
        "map" => {
            note_kind(step, StepKind::Map);
            note_over(step, flow_over(value));
        }
        "ai" => {
            note_kind(step, StepKind::Ai);
            let prompt = flow_prompt(value);
            set_summary(step, prompt);
        }
        _ => {}
    }
}

/// Apply a nested executor block (`run:` / `set:` / `map:` / `ai:`).
fn apply_step_block(step: &mut StepSpec, key: &str, raw: &[String]) {
    let pairs: Vec<(String, String)> = raw
        .iter()
        .filter_map(|line| split_field(line))
        .map(|(k, v)| (scalar(k), scalar(v)))
        .collect();
    match key {
        "run" => {
            note_kind(step, StepKind::Run);
            let command = raw
                .iter()
                .map(|line| line.strip_prefix("- ").unwrap_or(line).trim().to_string())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            set_summary(step, command);
        }
        "set" => {
            note_kind(step, StepKind::Set);
            let keys = pairs
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>()
                .join(", ");
            set_summary(step, keys);
        }
        "map" => {
            note_kind(step, StepKind::Map);
            let over = pairs
                .iter()
                .find(|(key, _)| key == "over")
                .map(|(_, value)| format!("over {value}"))
                .unwrap_or_default();
            note_over(step, over);
        }
        "ai" => {
            note_kind(step, StepKind::Ai);
            let prompt = pairs
                .iter()
                .find(|(key, _)| key == "prompt")
                .map(|(key, value)| {
                    if value == "|" || value == ">" {
                        first_block_line(raw, key).unwrap_or_default()
                    } else {
                        truncate(value, 70)
                    }
                })
                .unwrap_or_default();
            set_summary(step, prompt);
        }
        _ => {}
    }
}

/// The first line of a block scalar (`prompt: |`) inside a nested block.
fn first_block_line(raw: &[String], key: &str) -> Option<String> {
    let start = raw
        .iter()
        .position(|line| split_field(line).is_some_and(|(name, _)| name == key))?;
    raw.get(start + 1).map(|line| truncate(line, 70))
}

/// Record which executor a step uses, letting a fan-out win over the program it repeats.
fn note_kind(step: &mut StepSpec, kind: StepKind) {
    step.kind = match (step.kind, kind) {
        (StepKind::Unknown, kind) => kind,
        (StepKind::Run, StepKind::Map | StepKind::Set | StepKind::Ai) => kind,
        (current, _) => current,
    };
}

/// Keep the first meaningful summary; later ones are worth showing but not as the headline.
fn set_summary(step: &mut StepSpec, text: String) {
    if text.is_empty() {
        return;
    }
    if step.summary.is_empty() {
        step.summary = text;
    } else {
        step.notes.push(text);
    }
}

/// Record the collection a `map:` step fans out over.
fn note_over(step: &mut StepSpec, text: String) {
    if text.is_empty() {
        return;
    }
    if step.summary.is_empty() {
        step.summary = text;
    } else {
        step.notes.push(text);
    }
}

/// `[a, b]` or `a b` → a command line.
fn command_from(value: &str) -> String {
    match flow_sequence(value) {
        Some(items) => items.join(" "),
        None => scalar(value),
    }
}

/// Keys of an inline `set:` map.
fn flow_keys(value: &str) -> String {
    flow_body(value)
        .map(|inner| {
            split_flow_pairs(inner)
                .into_iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// `{ over: steps.x.json.items }` → `over steps.x.json.items`.
fn flow_over(value: &str) -> String {
    flow_body(value)
        .and_then(|inner| {
            split_flow_pairs(inner)
                .into_iter()
                .find(|(key, _)| key == "over")
                .map(|(_, value)| format!("over {value}"))
        })
        .unwrap_or_default()
}

/// `{ prompt: ... }` → a short prompt preview.
fn flow_prompt(value: &str) -> String {
    flow_body(value)
        .and_then(|inner| {
            split_flow_pairs(inner)
                .into_iter()
                .find(|(key, _)| key == "prompt")
                .map(|(_, value)| truncate(&value, 70))
        })
        .unwrap_or_default()
}

/// The body of an inline flow collection, when `value` is one.
fn flow_body(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    let inner = trimmed.strip_prefix('{')?.strip_suffix('}')?;
    Some(inner)
}

/// The items of an inline flow sequence, when `value` is one.
fn flow_sequence(value: &str) -> Option<Vec<String>> {
    let trimmed = value.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    Some(
        split_top_level(inner)
            .into_iter()
            .map(|item| scalar(&item))
            .filter(|item| !item.is_empty())
            .collect(),
    )
}

/// The key/value pairs of an inline flow map.
fn split_flow_pairs(inner: &str) -> Vec<(String, String)> {
    split_top_level(inner)
        .into_iter()
        .filter_map(|item| {
            let (key, value) = split_field(&item)?;
            Some((scalar(key), scalar(value)))
        })
        .collect()
}

/// Split on commas that are not inside quotes or nested collections.
fn split_top_level(inner: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for character in inner.chars() {
        match quote {
            Some(active) => {
                current.push(character);
                if character == active {
                    quote = None;
                }
            }
            None => match character {
                '"' | '\'' => {
                    quote = Some(character);
                    current.push(character);
                }
                '{' | '[' => {
                    depth += 1;
                    current.push(character);
                }
                '}' | ']' => {
                    depth -= 1;
                    current.push(character);
                }
                ',' if depth == 0 => items.push(std::mem::take(&mut current)),
                other => current.push(other),
            },
        }
    }
    if !current.trim().is_empty() {
        items.push(current);
    }
    items
}

/// Split `key: value` at the first colon outside quotes.
fn split_field(text: &str) -> Option<(&str, &str)> {
    let mut quote: Option<char> = None;
    for (offset, character) in text.char_indices() {
        match quote {
            Some(active) => {
                if character == active {
                    quote = None;
                }
            }
            None => match character {
                '"' | '\'' => quote = Some(character),
                ':' => {
                    let key = text[..offset].trim();
                    if key.is_empty() {
                        return None;
                    }
                    return Some((key, &text[offset + 1..]));
                }
                _ => {}
            },
        }
    }
    None
}

/// Drop a trailing `# comment` that is not inside quotes.
fn strip_comment(raw: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut previous = ' ';
    for (offset, character) in raw.char_indices() {
        match quote {
            Some(active) => {
                if character == active {
                    quote = None;
                }
            }
            None => match character {
                '"' | '\'' => quote = Some(character),
                '#' if previous.is_whitespace() => return &raw[..offset],
                _ => {}
            },
        }
        previous = character;
    }
    raw
}

/// Whether a value is a YAML block marker (`|`, `>`, `|-`, `>+`, …).
fn block_marker(value: &str) -> bool {
    let value = value.trim();
    if !(value.starts_with('|') || value.starts_with('>')) {
        return false;
    }
    value[1..]
        .chars()
        .all(|character| matches!(character, '-' | '+') || character.is_ascii_digit())
}

/// Read the indented lines a block marker introduces.
///
/// `|` keeps the line breaks and `>` folds them into spaces; either way the
/// block's own indentation is dropped, which is the text the reader expects.
fn read_block_scalar(lines: &[Line<'_>], parent: usize) -> (String, usize) {
    let marker = split_field(lines[parent].text)
        .map(|(_, value)| value.trim())
        .unwrap_or("|");
    let folded = marker.starts_with('>');
    let mut parts: Vec<String> = Vec::new();
    let mut next = parent + 1;
    while next < lines.len() && lines[next].indent > lines[parent].indent {
        parts.push(lines[next].text.to_string());
        next += 1;
    }
    let text = if folded {
        parts.join(" ")
    } else {
        parts.join("\n")
    };
    (text, next)
}

/// Trim, unquote and de-comment a scalar value.
fn scalar(raw: &str) -> String {
    let text = strip_comment(raw).trim();
    let unquoted = if text.len() >= 2 {
        let first = text.chars().next().unwrap_or(' ');
        let last = text.chars().last().unwrap_or(' ');
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            &text[first.len_utf8()..text.len() - last.len_utf8()]
        } else {
            text
        }
    } else {
        text
    };
    unquoted.trim().to_string()
}

/// YAML-ish truthiness for flags written as strings.
fn is_truthy(value: &str) -> bool {
    matches!(
        scalar(value).to_ascii_lowercase().as_str(),
        "true" | "yes" | "on" | "1"
    )
}

/// Shorten long single-line previews.
fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push('…');
    out
}

/// Read a string field from a JSON object.
fn string_at(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|entry| entry.as_str())
        .map(|text| text.to_string())
}

/// Render a JSON value as a single-line preview.
fn json_scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Array(items) => {
            items.iter().map(json_scalar).collect::<Vec<_>>().join(", ")
        }
        serde_json::Value::Object(map) => map.keys().cloned().collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// JSON `run:` → command line.
fn json_command(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Array(items) => {
            items.iter().map(json_scalar).collect::<Vec<_>>().join(" ")
        }
        other => json_scalar(other),
    }
}

/// JSON `set:` / `map:` / `ai:` → keys.
fn json_keys(value: &serde_json::Value) -> String {
    value
        .as_object()
        .map(|map| map.keys().cloned().collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

/// JSON `map:` → `over …` summary.
fn json_map_summary(value: &serde_json::Value) -> String {
    value
        .get("over")
        .map(|over| format!("over {}", json_scalar(over)))
        .unwrap_or_default()
}

/// JSON `ai:` → prompt preview.
fn json_ai_summary(value: &serde_json::Value) -> String {
    value
        .get("prompt")
        .map(|prompt| truncate(&json_scalar(prompt), 70))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN_EXAMPLE: &str = r#"
name: repo-health
description: 一句话说明
version: 1

inputs:
  repo: { description: 目录, required: false, default: "." }

vars:
  threshold: 0

defaults:
  run: { timeoutMs: 120000, cwd: ".", env: { TZ: UTC } }

steps:
  - id: counts
    desc: 统计
    run: [python, steps/count_files.py]
    save: counts

  - id: verdict
    when: steps.counts.json.files > 0
    set:
      summary: "{{ vars.counts.files }} files"
    save: report
"#;

    #[test]
    fn reads_identity_inputs_vars_and_steps() {
        let spec = parse(PLAN_EXAMPLE);
        assert_eq!(spec.name.as_deref(), Some("repo-health"));
        assert_eq!(spec.description.as_deref(), Some("一句话说明"));
        assert_eq!(spec.version.as_deref(), Some("1"));
        assert_eq!(spec.vars, vec![("threshold".to_string(), "0".to_string())]);
        assert_eq!(
            spec.inputs,
            vec![InputSpec {
                name: "repo".to_string(),
                description: "目录".to_string(),
                required: false,
                default: Some(".".to_string()),
            }]
        );
        assert_eq!(spec.steps.len(), 2);
    }

    #[test]
    fn reads_step_executors_and_guards() {
        let spec = parse(PLAN_EXAMPLE);
        let counts = &spec.steps[0];
        assert_eq!(counts.id, "counts");
        assert_eq!(counts.description, "统计");
        assert_eq!(counts.kind, StepKind::Run);
        assert_eq!(counts.summary, "python steps/count_files.py");
        assert_eq!(counts.save.as_deref(), Some("counts"));

        let verdict = &spec.steps[1];
        assert_eq!(verdict.kind, StepKind::Set);
        assert_eq!(verdict.when.as_deref(), Some("steps.counts.json.files > 0"));
        assert_eq!(verdict.summary, "summary");
        assert_eq!(verdict.save.as_deref(), Some("report"));
    }

    #[test]
    fn reads_block_inputs_and_step_flags() {
        let text = r#"
name: demo
inputs:
  target:
    description: which folder
    required: true
    default: /tmp # trailing comment
steps:
  - id: slow
    run: python steps/slow.py
    allowFailure: true
    retries: 2
    skip: never on ci
"#;
        let spec = parse(text);
        assert_eq!(
            spec.inputs,
            vec![InputSpec {
                name: "target".to_string(),
                description: "which folder".to_string(),
                required: true,
                default: Some("/tmp".to_string()),
            }]
        );
        let step = &spec.steps[0];
        assert_eq!(step.summary, "python steps/slow.py");
        assert_eq!(
            step.notes,
            vec!["allowFailure", "retries 2", "skip never on ci"]
        );
    }

    #[test]
    fn reads_block_executors() {
        let text = r#"
name: demo
steps:
  - id: fan
    map:
      over: steps.list.json.items
    run:
      - python
      - steps/item.py
  - id: ask
    ai:
      prompt: name the files
      schema: { type: object }
  - id: inline
    run: [node, steps/inline.ts]
"#;
        let spec = parse(text);
        assert_eq!(spec.steps[0].kind, StepKind::Map);
        assert_eq!(spec.steps[0].summary, "over steps.list.json.items");
        assert_eq!(spec.steps[0].notes, vec!["python steps/item.py"]);
        assert_eq!(spec.steps[1].kind, StepKind::Ai);
        assert_eq!(spec.steps[1].summary, "name the files");
        assert_eq!(spec.steps[2].summary, "node steps/inline.ts");
    }

    #[test]
    fn reads_a_block_scalar_prompt() {
        let text = r#"
name: demo
steps:
  - id: classify
    why: 需要判断
    ai:
      prompt: |
        把用户反馈归类为 bug / question / other。
        反馈：{{ steps.normalize.json.note }}
      schema:
        type: object
"#;
        let spec = parse(text);
        assert_eq!(spec.steps[0].kind, StepKind::Ai);
        assert_eq!(
            spec.steps[0].summary,
            "把用户反馈归类为 bug / question / other。"
        );
    }

    #[test]
    fn reads_a_block_scalar_description_and_command() {
        let text = r#"
name: demo
description: |
  第一行
  第二行
inputs:
  repo:
    description: |
      目录说明
    default: "."
steps:
  - id: build
    run: |
      npm run build
      npm test
"#;
        let spec = parse(text);
        assert_eq!(spec.description.as_deref(), Some("第一行\n第二行"));
        assert_eq!(spec.inputs[0].description, "目录说明");
        let step = &spec.steps[0];
        assert_eq!(step.kind, StepKind::Run);
        assert_eq!(step.summary, "npm run build npm test");
    }

    #[test]
    fn a_folded_block_scalar_joins_its_lines() {
        let text = "name: demo\ndescription: >\n  折成\n  一行\n";
        let spec = parse(text);
        assert_eq!(spec.description.as_deref(), Some("折成 一行"));
    }

    #[test]
    fn keeps_colons_inside_quotes() {
        let text = r#"
name: demo
description: "uses: colons, commas"
inputs:
  repo: { description: "dir: here", default: "a,b" }
"#;
        let spec = parse(text);
        assert_eq!(spec.description.as_deref(), Some("uses: colons, commas"));
        assert_eq!(spec.inputs[0].description, "dir: here");
        assert_eq!(spec.inputs[0].default.as_deref(), Some("a,b"));
    }

    #[test]
    fn reads_json_manifests() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{
                "name": "json-flow",
                "description": "json variant",
                "inputs": { "repo": { "description": "dir", "default": "." } },
                "vars": { "threshold": 3 },
                "steps": [
                    { "id": "counts", "run": ["python", "steps/count.py"], "save": "counts" },
                    { "id": "verdict", "set": { "summary": "x" } }
                ]
            }"#,
        )
        .unwrap();
        let spec = parse_json(&value);
        assert_eq!(spec.name.as_deref(), Some("json-flow"));
        assert_eq!(spec.inputs[0].default.as_deref(), Some("."));
        assert_eq!(spec.vars, vec![("threshold".to_string(), "3".to_string())]);
        assert_eq!(spec.steps[0].summary, "python steps/count.py");
        assert_eq!(spec.steps[1].kind, StepKind::Set);
        assert_eq!(spec.steps[1].summary, "summary");
    }
}
