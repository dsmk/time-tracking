use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use uuid::Uuid;

use crate::error::{AppError, Result, conflict, not_found, validation};
use crate::model::Entry;
use crate::timeutil::{MAX_TIMESTAMP_TEXT, from_storage, to_storage};

const ENTRY_COLUMNS: &str = "id, start_at, end_at, description, project, created_at, \
                             updated_at, revision, deleted_at, timezone";

/// Fields needed to create an entry, already normalized and validated.
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub description: String,
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub start_at: Timestamp,
    pub end_at: Option<Timestamp>,
    pub timezone: String,
}

/// Changes to apply to an existing entry. `None` means "leave alone".
#[derive(Debug, Clone, Default)]
pub struct EntryChanges {
    pub description: Option<String>,
    pub project: Option<Option<String>>,
    pub start_at: Option<Timestamp>,
    pub end_at: Option<Timestamp>,
    /// Replace the whole tag set.
    pub set_tags: Option<Vec<String>>,
    pub add_tags: Vec<String>,
    pub remove_tags: Vec<String>,
}

impl EntryChanges {
    pub fn is_empty(&self) -> bool {
        self.description.is_none()
            && self.project.is_none()
            && self.start_at.is_none()
            && self.end_at.is_none()
            && self.set_tags.is_none()
            && self.add_tags.is_empty()
            && self.remove_tags.is_empty()
    }
}

/// Query constraints for `tt log`. The window is the half-open instant range
/// `[from, to)`; an entry matches when any part of it falls inside.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub from: Option<Timestamp>,
    pub to: Option<Timestamp>,
    pub project: Option<String>,
    pub tag: Option<String>,
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub include_deleted: bool,
}

fn map_row(row: &Row<'_>) -> rusqlite::Result<Entry> {
    let parse = |raw: String| -> rusqlite::Result<Timestamp> {
        from_storage(&raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
    };
    let optional = |raw: Option<String>| -> rusqlite::Result<Option<Timestamp>> {
        raw.map(parse).transpose()
    };
    Ok(Entry {
        id: row.get("id")?,
        start_at: parse(row.get("start_at")?)?,
        end_at: optional(row.get("end_at")?)?,
        description: row.get("description")?,
        project: row.get("project")?,
        created_at: parse(row.get("created_at")?)?,
        updated_at: parse(row.get("updated_at")?)?,
        revision: row.get("revision")?,
        deleted_at: optional(row.get("deleted_at")?)?,
        timezone: row.get("timezone")?,
        tags: Vec::new(),
    })
}

/// Load the tag lists for a batch of entries in one query.
fn hydrate_tags(conn: &Connection, entries: &mut [Entry]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let placeholders = vec!["?"; entries.len()].join(", ");
    let sql = format!(
        "SELECT entry_id, tag FROM entry_tags WHERE entry_id IN ({placeholders}) \
         ORDER BY entry_id, rowid"
    );
    let mut stmt = conn.prepare(&sql)?;
    let ids: Vec<&dyn rusqlite::ToSql> =
        entries.iter().map(|e| &e.id as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(ids.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (entry_id, tag) = row?;
        if let Some(entry) = entries.iter_mut().find(|e| e.id == entry_id) {
            entry.tags.push(tag);
        }
    }
    Ok(())
}

/// The running timer, if one exists.
pub fn running(conn: &Connection) -> Result<Option<Entry>> {
    let sql = format!(
        "SELECT {ENTRY_COLUMNS} FROM entries WHERE end_at IS NULL AND deleted_at IS NULL"
    );
    let mut entry = conn.query_row(&sql, [], map_row).optional()?;
    if let Some(e) = entry.as_mut() {
        hydrate_tags(conn, std::slice::from_mut(e))?;
    }
    Ok(entry)
}

/// Entries that overlap the half-open interval `[start, end)`, ignoring
/// `exclude_id`. A running entry is treated as open-ended, so nothing may be
/// recorded after a running timer started.
pub fn overlapping(
    conn: &Connection,
    exclude_id: Option<&str>,
    start: Timestamp,
    end: Option<Timestamp>,
) -> Result<Vec<Entry>> {
    let candidate_end =
        end.map(to_storage).unwrap_or_else(|| MAX_TIMESTAMP_TEXT.to_string());
    let sql = format!(
        "SELECT {ENTRY_COLUMNS} FROM entries \
         WHERE deleted_at IS NULL \
           AND (?1 IS NULL OR id <> ?1) \
           AND start_at < ?2 \
           AND COALESCE(end_at, '{MAX_TIMESTAMP_TEXT}') > ?3 \
         ORDER BY start_at, id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![exclude_id, candidate_end, to_storage(start)], map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut entries = rows;
    hydrate_tags(conn, &mut entries)?;
    Ok(entries)
}

fn overlap_error(conflicts: &[Entry]) -> AppError {
    let first = &conflicts[0];
    let more = if conflicts.len() > 1 {
        format!(" (and {} more)", conflicts.len() - 1)
    } else {
        String::new()
    };
    conflict(format!(
        "the time range overlaps entry {} \"{}\"{more}",
        first.short_id(),
        first.description
    ))
    .with_hint("adjust the times, or pass --allow-overlap to record them anyway")
}

/// Reject the proposed interval if it collides with an existing entry.
pub fn ensure_no_overlap(
    conn: &Connection,
    exclude_id: Option<&str>,
    start: Timestamp,
    end: Option<Timestamp>,
    allow_overlap: bool,
) -> Result<()> {
    if allow_overlap {
        return Ok(());
    }
    let conflicts = overlapping(conn, exclude_id, start, end)?;
    if conflicts.is_empty() {
        return Ok(());
    }
    Err(overlap_error(&conflicts))
}

fn validate_span(start: Timestamp, end: Option<Timestamp>) -> Result<()> {
    if let Some(end) = end
        && end <= start
    {
        return Err(validation(format!(
            "end time {} must be later than start time {}",
            to_storage(end),
            to_storage(start)
        )));
    }
    Ok(())
}

/// Translate the single-running-timer unique index into a readable error.
fn map_write_error(err: rusqlite::Error) -> AppError {
    let text = err.to_string();
    if text.contains("entries_one_running") || text.contains("running_marker") {
        return conflict(
            "another timer was started concurrently; only one timer may run at a time",
        )
        .with_hint("run `tt status` to see the running timer, then `tt stop`");
    }
    AppError::from(err)
}

fn insert_entry(tx: &Transaction<'_>, new: &NewEntry, now: Timestamp) -> Result<Entry> {
    validate_span(new.start_at, new.end_at)?;
    let id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO entries (id, start_at, end_at, description, project, created_at, \
         updated_at, revision, deleted_at, timezone) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 1, NULL, ?7)",
        params![
            id,
            to_storage(new.start_at),
            new.end_at.map(to_storage),
            new.description,
            new.project,
            to_storage(now),
            new.timezone,
        ],
    )
    .map_err(map_write_error)?;

    for tag in &new.tags {
        tx.execute("INSERT INTO entry_tags (entry_id, tag) VALUES (?1, ?2)", params![id, tag])
            .map_err(map_write_error)?;
    }

    Ok(Entry {
        id,
        start_at: new.start_at,
        end_at: new.end_at,
        description: new.description.clone(),
        project: new.project.clone(),
        created_at: now,
        updated_at: now,
        revision: 1,
        deleted_at: None,
        timezone: new.timezone.clone(),
        tags: new.tags.clone(),
    })
}

/// Start a timer. Fails, without changing anything, if a timer is already
/// running or the start time overlaps an existing entry.
pub fn start(
    conn: &mut Connection,
    new: NewEntry,
    now: Timestamp,
    allow_overlap: bool,
) -> Result<Entry> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(active) = running(&tx)? {
        // Report the start in the timezone the entry was created in, matching
        // the local format every other human-readable message uses.
        let started = match jiff::tz::TimeZone::get(&active.timezone) {
            Ok(tz) => crate::timeutil::format_local_datetime(
                active.start_at,
                &tz,
                crate::timeutil::Precision::Minute,
            ),
            Err(_) => to_storage(active.start_at),
        };
        return Err(conflict(format!(
            "a timer is already running: {} \"{}\" started {started}",
            active.short_id(),
            active.description,
        ))
        .with_hint(format!("stop it first with `tt stop`, or discard it with `tt delete {}`", active.short_id())));
    }
    ensure_no_overlap(&tx, None, new.start_at, None, allow_overlap)?;
    let entry = insert_entry(&tx, &new, now)?;
    tx.commit()?;
    Ok(entry)
}

/// Create an already-completed entry.
pub fn add(
    conn: &mut Connection,
    new: NewEntry,
    now: Timestamp,
    allow_overlap: bool,
) -> Result<Entry> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_span(new.start_at, new.end_at)?;
    ensure_no_overlap(&tx, None, new.start_at, new.end_at, allow_overlap)?;
    let entry = insert_entry(&tx, &new, now)?;
    tx.commit()?;
    Ok(entry)
}

/// Stop the running timer at `at`. The timer is left running on any failure.
pub fn stop(
    conn: &mut Connection,
    at: Timestamp,
    now: Timestamp,
    allow_overlap: bool,
) -> Result<Entry> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(mut entry) = running(&tx)? else {
        return Err(conflict("no timer is running")
            .with_hint("start one with `tt start \"description\"`"));
    };
    if at <= entry.start_at {
        return Err(validation(format!(
            "stop time {} must be later than the start time {}",
            to_storage(at),
            to_storage(entry.start_at)
        )));
    }
    ensure_no_overlap(&tx, Some(&entry.id), entry.start_at, Some(at), allow_overlap)?;

    tx.execute(
        "UPDATE entries SET end_at = ?1, updated_at = ?2, revision = revision + 1 WHERE id = ?3",
        params![to_storage(at), to_storage(now), entry.id],
    )
    .map_err(map_write_error)?;
    tx.commit()?;

    entry.end_at = Some(at);
    entry.updated_at = now;
    entry.revision += 1;
    Ok(entry)
}

/// Resolve a full UUID or an unambiguous prefix.
pub fn find(conn: &Connection, reference: &str, include_deleted: bool) -> Result<Entry> {
    let prefix = reference.trim().to_ascii_lowercase();
    if prefix.is_empty() {
        return Err(validation("an entry id is required"));
    }
    if !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(validation(format!(
            "{reference:?} is not a valid entry id or id prefix"
        ))
        .with_hint("ids are UUIDs; `tt log` prints a short id for each entry"));
    }

    let sql = format!(
        "SELECT {ENTRY_COLUMNS} FROM entries WHERE substr(lower(id), 1, ?1) = ?2 \
         ORDER BY start_at DESC, id LIMIT 3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut matches = stmt
        .query_map(params![prefix.chars().count() as i64, prefix], map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    if !include_deleted {
        let had_deleted = matches.iter().any(|e| e.deleted_at.is_some());
        matches.retain(|e| e.deleted_at.is_none());
        if matches.is_empty() && had_deleted {
            return Err(not_found(format!("entry {reference} has been deleted"))
                .with_hint("pass --include-deleted to inspect deleted entries"));
        }
    }

    match matches.len() {
        0 => Err(not_found(format!("no entry matches {reference:?}"))),
        1 => {
            let mut entry = matches.remove(0);
            hydrate_tags(conn, std::slice::from_mut(&mut entry))?;
            Ok(entry)
        }
        _ => {
            let shown: Vec<String> = matches
                .iter()
                .take(3)
                .map(|e| format!("{} \"{}\"", e.short_id(), e.description))
                .collect();
            Err(conflict(format!(
                "{reference:?} matches more than one entry: {}",
                shown.join(", ")
            ))
            .with_hint("supply a longer id prefix"))
        }
    }
}

/// List entries, newest first.
pub fn list(conn: &Connection, filter: &ListFilter) -> Result<Vec<Entry>> {
    let mut sql = format!("SELECT {ENTRY_COLUMNS} FROM entries WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if !filter.include_deleted {
        sql.push_str(" AND deleted_at IS NULL");
    }
    if let Some(to) = filter.to {
        args.push(Box::new(to_storage(to)));
        sql.push_str(&format!(" AND start_at < ?{}", args.len()));
    }
    if let Some(from) = filter.from {
        args.push(Box::new(to_storage(from)));
        sql.push_str(&format!(
            " AND COALESCE(end_at, '{MAX_TIMESTAMP_TEXT}') > ?{}",
            args.len()
        ));
    }
    if let Some(project) = &filter.project {
        args.push(Box::new(project.clone()));
        sql.push_str(&format!(" AND project IS NOT NULL AND project = ?{} COLLATE NOCASE", args.len()));
    }
    if let Some(tag) = &filter.tag {
        args.push(Box::new(tag.clone()));
        sql.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM entry_tags t WHERE t.entry_id = entries.id \
              AND t.tag = ?{} COLLATE NOCASE)",
            args.len()
        ));
    }
    if let Some(search) = &filter.search {
        args.push(Box::new(format!("%{}%", escape_like(search))));
        sql.push_str(&format!(
            " AND lower(description) LIKE lower(?{}) ESCAPE '\\'",
            args.len()
        ));
    }

    // start_at DESC puts the newest first; id breaks ties so output order is
    // deterministic for identical start times.
    sql.push_str(" ORDER BY start_at DESC, id DESC");
    if let Some(limit) = filter.limit {
        args.push(Box::new(limit));
        sql.push_str(&format!(" LIMIT ?{}", args.len()));
    }

    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
    let mut entries =
        stmt.query_map(refs.as_slice(), map_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    hydrate_tags(conn, &mut entries)?;
    Ok(entries)
}

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Apply changes to an entry, validating the result as a whole. On any error
/// the transaction rolls back and the entry is left exactly as it was.
pub fn edit(
    conn: &mut Connection,
    reference: &str,
    changes: &EntryChanges,
    now: Timestamp,
    allow_overlap: bool,
) -> Result<Entry> {
    if changes.is_empty() {
        return Err(validation("no changes were requested")
            .with_hint("supply at least one of --description, --project, --clear-project, \
                        --start, --end, --tag, --add-tag, or --remove-tag"));
    }

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut entry = find(&tx, reference, false)?;
    let was_running = entry.is_running();

    if changes.end_at.is_some() && was_running {
        return Err(validation(format!(
            "entry {} is still running, so it has no end time to replace",
            entry.short_id()
        ))
        .with_hint("stop the timer first with `tt stop --at TIME`"));
    }

    if let Some(description) = &changes.description {
        entry.description = description.clone();
    }
    if let Some(project) = &changes.project {
        entry.project = project.clone();
    }
    if let Some(start) = changes.start_at {
        entry.start_at = start;
    }
    if let Some(end) = changes.end_at {
        entry.end_at = Some(end);
    }

    validate_span(entry.start_at, entry.end_at)?;
    ensure_no_overlap(&tx, Some(&entry.id), entry.start_at, entry.end_at, allow_overlap)?;

    // Resolve the final tag set: an explicit --tag list replaces everything,
    // then --add-tag and --remove-tag are applied on top.
    let mut tags = changes.set_tags.clone().unwrap_or_else(|| entry.tags.clone());
    tags.extend(changes.add_tags.iter().cloned());
    tags = crate::model::dedupe_tags(tags);
    for removal in &changes.remove_tags {
        tags.retain(|existing| !existing.eq_ignore_ascii_case(removal));
    }
    let tags_changed = tags != entry.tags;

    tx.execute(
        "UPDATE entries SET start_at = ?1, end_at = ?2, description = ?3, project = ?4, \
         updated_at = ?5, revision = revision + 1 WHERE id = ?6",
        params![
            to_storage(entry.start_at),
            entry.end_at.map(to_storage),
            entry.description,
            entry.project,
            to_storage(now),
            entry.id,
        ],
    )
    .map_err(map_write_error)?;

    if tags_changed {
        tx.execute("DELETE FROM entry_tags WHERE entry_id = ?1", params![entry.id])?;
        for tag in &tags {
            tx.execute(
                "INSERT INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
                params![entry.id, tag],
            )
            .map_err(map_write_error)?;
        }
    }
    tx.commit()?;

    entry.tags = tags;
    entry.updated_at = now;
    entry.revision += 1;
    Ok(entry)
}

/// Soft-delete an entry: set `deleted_at`, bump the revision, keep the row.
pub fn delete(conn: &mut Connection, reference: &str, now: Timestamp) -> Result<Entry> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut entry = find(&tx, reference, false)?;
    tx.execute(
        "UPDATE entries SET deleted_at = ?1, updated_at = ?1, revision = revision + 1 \
         WHERE id = ?2",
        params![to_storage(now), entry.id],
    )
    .map_err(map_write_error)?;
    tx.commit()?;
    entry.deleted_at = Some(now);
    entry.updated_at = now;
    entry.revision += 1;
    Ok(entry)
}

/// Pairs of live entries whose intervals overlap, used for report warnings.
pub fn overlap_pairs(entries: &[Entry], now: Timestamp) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for (i, a) in entries.iter().enumerate() {
        for b in entries.iter().skip(i + 1) {
            let a_end = a.effective_end(now);
            let b_end = b.effective_end(now);
            if a.start_at < b_end && b.start_at < a_end {
                pairs.push((a.id.clone(), b.id.clone()));
            }
        }
    }
    pairs
}
