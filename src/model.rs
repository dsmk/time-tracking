use jiff::Timestamp;

use crate::error::{Result, validation};

/// A single tracked interval. `end_at` is `None` while the timer runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub start_at: Timestamp,
    pub end_at: Option<Timestamp>,
    pub description: String,
    pub project: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub revision: i64,
    pub deleted_at: Option<Timestamp>,
    pub timezone: String,
    pub tags: Vec<String>,
}

impl Entry {
    pub fn is_running(&self) -> bool {
        self.end_at.is_none() && self.deleted_at.is_none()
    }

    /// The instant this entry currently ends at: its recorded end, or `now`
    /// while it is still running.
    pub fn effective_end(&self, now: Timestamp) -> Timestamp {
        self.end_at.unwrap_or(now)
    }

    /// Elapsed seconds, measured from the stored instants and never rounded.
    pub fn duration_seconds(&self, now: Timestamp) -> i64 {
        let end = self.effective_end(now).as_second();
        (end - self.start_at.as_second()).max(0)
    }

    /// The abbreviated id shown in list output.
    pub fn short_id(&self) -> String {
        self.id.chars().take(8).collect()
    }

    pub fn project_label(&self) -> &str {
        self.project.as_deref().unwrap_or("")
    }
}

/// Descriptions are required, trimmed, and must contain something.
pub fn normalize_description(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(validation("description must not be empty")
            .with_hint("pass the description as the first positional argument, for example: \
                        tt start \"Design review\""));
    }
    Ok(trimmed.to_string())
}

/// Projects are trimmed; an all-whitespace project means "no project".
pub fn normalize_project(raw: Option<&str>) -> Result<Option<String>> {
    let Some(value) = raw else { return Ok(None) };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(validation(format!(
            "project {value:?} contains control characters"
        )));
    }
    Ok(Some(trimmed.to_string()))
}

/// Normalize one tag: trim, drop a leading `#`, and reject anything that would
/// not round-trip through `--tag` or `#tag` syntax.
pub fn normalize_tag(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_start_matches('#').trim();
    if trimmed.is_empty() {
        return Err(validation(format!("tag {raw:?} is empty"))
            .with_hint("tags are non-empty words, for example --tag incident or #incident"));
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(validation(format!("tag {raw:?} must not contain whitespace"))
            .with_hint("use a single word, or join words with - or _"));
    }
    Ok(trimmed.to_string())
}

/// Pull `#tag` tokens out of a description. The `#tag` text stays in the
/// description; this only mirrors it into the entry's tag list.
pub fn tags_in_description(description: &str) -> Vec<String> {
    let mut found = Vec::new();
    let chars: Vec<char> = description.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let at_boundary = i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(';
        if chars[i] == '#' && at_boundary {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '-' || chars[j] == '_')
            {
                j += 1;
            }
            if j > i + 1 {
                found.push(chars[i + 1..j].iter().collect::<String>());
            }
            i = j;
            continue;
        }
        i += 1;
    }
    found
}

/// Combine tags, comparing case-insensitively for duplicates and keeping the
/// first spelling seen. Order is the order of first appearance.
pub fn dedupe_tags(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        if !out.iter().any(|existing| existing.eq_ignore_ascii_case(&tag)) {
            out.push(tag);
        }
    }
    out
}

/// Validate and normalize the complete tag set for an entry: explicit tags
/// first, then any `#tag` mentioned in the description.
pub fn collect_tags(explicit: &[String], description: &str) -> Result<Vec<String>> {
    let mut all = Vec::new();
    for raw in explicit {
        all.push(normalize_tag(raw)?);
    }
    for raw in tags_in_description(description) {
        all.push(normalize_tag(&raw)?);
    }
    Ok(dedupe_tags(all))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(raw: &str) -> Timestamp {
        raw.parse().unwrap()
    }

    #[test]
    fn descriptions_are_trimmed_and_required() {
        assert_eq!(normalize_description("  Design review \n").unwrap(), "Design review");
        assert!(normalize_description("").is_err());
        assert!(normalize_description("   \t ").is_err());
    }

    #[test]
    fn projects_are_trimmed_and_blank_means_none() {
        assert_eq!(normalize_project(Some("  alpha ")).unwrap().as_deref(), Some("alpha"));
        assert_eq!(normalize_project(Some("   ")).unwrap(), None);
        assert_eq!(normalize_project(None).unwrap(), None);
    }

    #[test]
    fn tags_are_trimmed_and_may_be_written_with_or_without_a_hash() {
        assert_eq!(normalize_tag(" incident ").unwrap(), "incident");
        assert_eq!(normalize_tag("#incident").unwrap(), "incident");
        assert!(normalize_tag("#").is_err());
        assert!(normalize_tag("two words").is_err());
        assert!(normalize_tag("").is_err());
    }

    #[test]
    fn duplicate_tags_are_detected_case_insensitively_keeping_first_spelling() {
        let tags = dedupe_tags(
            ["Incident", "incident", "INCIDENT", "oncall"].map(String::from).into_iter(),
        );
        assert_eq!(tags, vec!["Incident".to_string(), "oncall".to_string()]);
    }

    #[test]
    fn hash_tags_are_extracted_from_the_description() {
        assert_eq!(
            tags_in_description("Investigate #incident while #on-call"),
            vec!["incident".to_string(), "on-call".to_string()]
        );
        // A hash inside a word is not a tag, and a bare hash is ignored.
        assert!(tags_in_description("issue#42 and # alone").is_empty());
        assert_eq!(tags_in_description("#start of line"), vec!["start".to_string()]);
    }

    #[test]
    fn flag_tags_and_hash_tags_merge_without_duplicates() {
        let tags =
            collect_tags(&["oncall".to_string(), "#Incident".to_string()], "Investigate #incident")
                .unwrap();
        assert_eq!(tags, vec!["oncall".to_string(), "Incident".to_string()]);
    }

    #[test]
    fn description_text_keeps_its_hash_tags() {
        // collect_tags reads the description but callers store it verbatim.
        let description = normalize_description("Investigate #incident").unwrap();
        assert_eq!(description, "Investigate #incident");
    }

    #[test]
    fn running_entries_measure_duration_through_now() {
        let entry = Entry {
            id: "11112222-3333-4444-5555-666677778888".into(),
            start_at: ts("2026-10-27T13:00:00Z"),
            end_at: None,
            description: "x".into(),
            project: None,
            created_at: ts("2026-10-27T13:00:00Z"),
            updated_at: ts("2026-10-27T13:00:00Z"),
            revision: 1,
            deleted_at: None,
            timezone: "UTC".into(),
            tags: vec![],
        };
        assert!(entry.is_running());
        assert_eq!(entry.duration_seconds(ts("2026-10-27T14:30:00Z")), 5400);
        assert_eq!(entry.short_id(), "11112222");

        let stopped = Entry { end_at: Some(ts("2026-10-27T13:45:00Z")), ..entry };
        assert!(!stopped.is_running());
        // A stopped entry ignores `now`.
        assert_eq!(stopped.duration_seconds(ts("2026-10-27T20:00:00Z")), 2700);
    }
}
