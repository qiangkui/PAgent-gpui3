//! Finding the JSON document an engine printed among whatever else it logged.
//!
//! Every engine answer is one JSON object — a run summary, a compact checkpoint,
//! a model list. A chatty build may print progress lines first, so the document
//! has to be picked out from the end: taking the first `{` in the stream would
//! read a log line as the result and report a successful run as a failure.

/// The JSON object `text` holds, preferring the last complete document.
///
/// Plain one-line documents are tried first, newest last, because that is how
/// the engine prints them. A pretty-printed document that spans lines is then
/// found by trying every `{` from the end.
pub fn object(text: &str) -> Option<serde_json::Value> {
    for line in text.lines().rev() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line)
            && value.is_object()
        {
            return Some(value);
        }
    }
    let mut starts: Vec<usize> = text.match_indices('{').map(|(at, _)| at).collect();
    while let Some(start) = starts.pop() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text[start..])
            && value.is_object()
        {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_last_document_when_a_log_line_came_first() {
        let text = "{\"level\":\"info\",\"msg\":\"starting\"}\n{\"status\":\"ok\",\"runId\":\"run-9\"}";
        let found = object(text).expect("a summary");
        assert_eq!(found["runId"], "run-9");
        // The log line was not mistaken for the summary.
        assert!(found.get("level").is_none());
    }

    #[test]
    fn finds_a_pretty_printed_document_at_the_end() {
        let text = "warming up\n{\n  \"status\": \"ok\",\n  \"steps\": []\n}";
        let found = object(text).expect("a summary");
        assert_eq!(found["status"], "ok");
    }

    #[test]
    fn reports_nothing_when_the_output_holds_no_object() {
        assert!(object("boom").is_none());
        assert!(object("").is_none());
        assert!(object("[1, 2]").is_none());
    }
}