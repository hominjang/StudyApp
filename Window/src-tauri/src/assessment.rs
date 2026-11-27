//! V2 commands. The active v1 app DB is never upgraded by this service.
use crate::{
    error::{AppError, AppResult},
    exam::validate_exam_id,
    learning_format::Criteria,
    validation::{is_unanswered, is_valid_answer, is_valid_id},
};
use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use uuid::Uuid;
mod foundation;
mod initialize;
mod progress;
mod runtime;
mod types;
use progress::{checked_progress, compute_progress, save_progress};
pub(crate) use runtime::session as v2_session;
pub use types::*;

const MAX_REV: u64 = 9_007_199_254_740_991;
fn sql_revision(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    if value < 0 || value as u64 > MAX_REV {
        return Err(rusqlite::Error::IntegralValueOutOfRange(index, value));
    }
    Ok(value as u64)
}
pub struct AssessmentStore {
    pub(crate) db: Connection,
    learner: String,
}
fn corrupt() -> AppError {
    AppError::new(
        "ProgressSourceCorrupt",
        "관련 학습 이력을 확인하지 못했어요. 원본을 보존하며 저장을 중단했어요.",
        false,
    )
}
fn conflict(code: &'static str) -> AppError {
    AppError::new(
        code,
        "저장된 상태가 바뀌었어요. 최신 상태를 확인한 뒤 다시 시도해주세요.",
        false,
    )
}
fn decode<T: serde::de::DeserializeOwned>(s: &str) -> AppResult<T> {
    serde_json::from_str(s).map_err(|_| corrupt())
}
fn json<T: serde::Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|_| corrupt())
}
fn uuid(value: &str) -> AppResult<()> {
    validate_exam_id(value).map_err(|_| AppError::validation("id"))
}

impl AssessmentStore {
    pub fn open(path: &Path) -> AppResult<Self> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        db.busy_timeout(Duration::from_secs(3))?;
        let meta = |key: &str| -> AppResult<String> {
            Ok(
                db.query_row("SELECT value FROM app_metadata WHERE key=?1", [key], |r| {
                    r.get(0)
                })?,
            )
        };
        if meta("application_id")? != "StudyApp" {
            return Err(AppError::inconsistent());
        }
        if meta("db_schema_version")? != "2" {
            return Err(AppError::new(
                "UnsupportedDbVersion",
                "이 기능은 확장 저장소에서 사용할 수 있어요. 현재 저장소는 변경하지 않았어요.",
                false,
            ));
        }
        if meta("initial_data_installed")? != "1" {
            return Err(AppError::inconsistent());
        }
        let learner = meta("local_learner_id")?;
        uuid(&learner)?;
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM learners WHERE learner_id=?1)",
            [&learner],
            |r| r.get(0),
        )?;
        let profiles: i64 = db.query_row("SELECT COUNT(*) FROM learners", [], |r| r.get(0))?;
        if !exists || profiles != 1 {
            return Err(AppError::inconsistent());
        }
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
        Ok(Self { db, learner })
    }

    pub fn assessment_view(&mut self, id: &str) -> AppResult<AssessmentView> {
        uuid(id)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let (revision, count) = completed_exam(&tx, &self.learner, id)?;
        let positions = tx
            .prepare("SELECT position FROM exam_responses WHERE exam_id=?1 ORDER BY position")?
            .query_map([id], |r| r.get::<_, u32>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if positions.len() != count as usize
            || positions.iter().enumerate().any(|(i, p)| i != *p as usize)
        {
            return Err(corrupt());
        }
        let mut counts = AssessmentCounts::default();
        let mut items = Vec::new();
        for position in positions {
            let r = load_response(&tx, id, position)?;
            match r.judgement.as_str() {
                "correct" => counts.correct += 1,
                "partial" => counts.partial += 1,
                "incorrect" => counts.incorrect += 1,
                "ungraded" => counts.ungraded += 1,
                _ => return Err(corrupt()),
            }
            let progress = checked_progress(&tx, &self.learner, &r.dataset, &r.item)?;
            items.push(AssessmentItem {
                position,
                question: r.question,
                response_text: r.text,
                criteria: r.criteria,
                original_judgement: r.original,
                judgement: r.judgement,
                incorrect_reason: r.reason,
                assessment_revision: r.revision,
                is_review_target: progress.target,
                correct_streak: progress.streak,
                progress_revision: progress.revision,
            });
        }
        let view = AssessmentView {
            exam_id: id.into(),
            result_revision: revision,
            assessment_status: if counts.ungraded > 0 {
                "pending"
            } else {
                "complete"
            }
            .into(),
            counts,
            items,
        };
        tx.commit()?;
        Ok(view)
    }

    pub fn set_assessment(&mut self, request: SetAssessmentRequest) -> AppResult<MutationResult> {
        uuid(&request.exam_id)?;
        uuid(&request.request_id)?;
        if request.expected_revision > MAX_REV
            || request.note.as_ref().is_some_and(|s| s.len() > 2048)
        {
            return Err(AppError::validation("revision/note"));
        }
        let hash = format!("{:x}", Sha256::digest(json(&request)?.as_bytes()));
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = retry(
            &tx,
            &self.learner,
            &request.request_id,
            "set_response_assessment",
            &hash,
        )? {
            tx.commit()?;
            return Ok(result);
        }
        let (result_revision, count) = completed_exam(&tx, &self.learner, &request.exam_id)?;
        if request.position >= count {
            return Err(AppError::validation("position"));
        }
        let response = load_response(&tx, &request.exam_id, request.position)?;
        let before = checked_progress(&tx, &self.learner, &response.dataset, &response.item)?;
        if response.revision != request.expected_revision {
            return Err(conflict("AssessmentConflict"));
        }
        if is_unanswered(&response.text) {
            return Err(AppError::new(
                "AssessmentNotAllowed",
                "미응답은 평가를 정정할 수 없어요.",
                false,
            ));
        }
        let judgement = request.judgement.text();
        let reason = if judgement == "incorrect" {
            Some("user")
        } else {
            None
        };
        let unchanged = response.judgement == judgement
            && response.reason.as_deref() == reason
            && response.note == request.note;
        let result = if unchanged {
            MutationResult {
                outcome: "unchanged".into(),
                assessment_revision: Some(response.revision),
                progress_revision: before.revision,
                result_revision: Some(result_revision),
            }
        } else {
            let next = response
                .revision
                .checked_add(1)
                .filter(|n| *n <= MAX_REV)
                .ok_or_else(corrupt)?;
            let event = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO assessment_events VALUES (?1,?2,?3,?4,?5,?6,?7,'user',?8,?9,?10)",
                params![
                    event,
                    request.exam_id,
                    request.position,
                    next as i64,
                    response.judgement,
                    judgement,
                    reason,
                    Utc::now().to_rfc3339(),
                    request.note,
                    request.request_id
                ],
            )?;
            tx.execute("UPDATE response_assessments SET judgement=?1,incorrect_reason=?2,assessment_revision=?3,latest_event_id=?4 WHERE exam_id=?5 AND position=?6",params![judgement,reason,next as i64,event,request.exam_id,request.position])?;
            let after = compute_progress(&tx, &self.learner, &response.dataset, &response.item)?;
            let progress_revision = save_progress(
                &tx,
                &self.learner,
                &response.dataset,
                &response.item,
                after,
                before.revision,
            )?;
            let revision = result_revision
                .checked_add(1)
                .filter(|n| *n <= MAX_REV)
                .ok_or_else(corrupt)?;
            tx.execute(
                "UPDATE exams SET result_revision=?1 WHERE exam_id=?2",
                params![revision as i64, request.exam_id],
            )?;
            MutationResult {
                outcome: "saved".into(),
                assessment_revision: Some(next),
                progress_revision,
                result_revision: Some(revision),
            }
        };
        remember(
            &tx,
            &self.learner,
            &request.request_id,
            "set_response_assessment",
            &hash,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn add_review(&mut self, request: AddReviewRequest) -> AppResult<MutationResult> {
        uuid(&request.request_id)?;
        if !is_valid_id(&request.dataset_id)
            || !is_valid_id(&request.item_id)
            || request.expected_progress_revision > MAX_REV
        {
            return Err(AppError::validation("review_target"));
        }
        let hash = format!("{:x}", Sha256::digest(json(&request)?.as_bytes()));
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = retry(
            &tx,
            &self.learner,
            &request.request_id,
            "add_review_target",
            &hash,
        )? {
            tx.commit()?;
            return Ok(result);
        }
        let active: Option<i64> = tx
            .query_row(
                "SELECT is_active FROM items WHERE dataset_id=?1 AND item_id=?2",
                params![request.dataset_id, request.item_id],
                |r| r.get(0),
            )
            .optional()?;
        if active != Some(1) {
            return Err(AppError::new(
                "LinkedItemUnavailable",
                "복습할 문항이 없거나 비활성 상태예요.",
                false,
            ));
        }
        let before = checked_progress(&tx, &self.learner, &request.dataset_id, &request.item_id)?;
        if before.revision != request.expected_progress_revision {
            return Err(conflict("ProgressConflict"));
        }
        let revision = if before.target {
            before.revision
        } else {
            let sequence: i64 = tx.query_row(
                "SELECT COALESCE(MAX(event_seq),0)+1 FROM learning_events WHERE learner_id=?1",
                [&self.learner],
                |r| r.get(0),
            )?;
            if sequence < 1 || sequence as u64 > MAX_REV {
                return Err(corrupt());
            }
            tx.execute(
                "INSERT INTO learning_events VALUES (?1,?2,?3,'manual_review_add',NULL,?4,?5,?6)",
                params![
                    self.learner,
                    sequence,
                    Uuid::new_v4().to_string(),
                    request.dataset_id,
                    request.item_id,
                    Utc::now().to_rfc3339()
                ],
            )?;
            let after =
                compute_progress(&tx, &self.learner, &request.dataset_id, &request.item_id)?;
            save_progress(
                &tx,
                &self.learner,
                &request.dataset_id,
                &request.item_id,
                after,
                before.revision,
            )?
        };
        let result = MutationResult {
            outcome: if before.target { "unchanged" } else { "saved" }.into(),
            assessment_revision: None,
            progress_revision: revision,
            result_revision: None,
        };
        remember(
            &tx,
            &self.learner,
            &request.request_id,
            "add_review_target",
            &hash,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }
}

fn completed_exam(db: &Connection, learner: &str, id: &str) -> AppResult<(u64, u32)> {
    let row: Option<(String, String, u64, u32)> = db
        .query_row(
            "SELECT learner_id,status,result_revision,question_count FROM exams WHERE exam_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, sql_revision(r, 2)?, r.get(3)?)),
        )
        .optional()?;
    let (owner, status, revision, count) =
        row.ok_or_else(|| AppError::new("NotFound", "시험을 찾지 못했어요.", false))?;
    if owner != learner {
        return Err(AppError::new(
            "OwnershipMismatch",
            "이 학습 공간의 시험이 아니에요.",
            false,
        ));
    }
    if status != "completed" {
        return Err(AppError::new(
            "InvalidState",
            "제출 처리가 완료된 시험에서 평가할 수 있어요.",
            false,
        ));
    }
    if revision > MAX_REV || count == 0 {
        return Err(corrupt());
    }
    Ok((revision, count))
}

pub(crate) fn history_counts(
    db: &Connection,
    learner: &str,
    id: &str,
) -> AppResult<AssessmentCounts> {
    let (_, count) = completed_exam(db, learner, id)?;
    let positions: Vec<u32> = db
        .prepare("SELECT position FROM exam_responses WHERE exam_id=?1 ORDER BY position")?
        .query_map([id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    if positions.len() != count as usize
        || positions.iter().enumerate().any(|(i, p)| i != *p as usize)
    {
        return Err(corrupt());
    }
    let mut counts = AssessmentCounts::default();
    for position in positions {
        let response = load_response(db, id, position)?;
        match response.judgement.as_str() {
            "correct" => counts.correct += 1,
            "partial" => counts.partial += 1,
            "incorrect" => counts.incorrect += 1,
            "ungraded" => counts.ungraded += 1,
            _ => return Err(corrupt()),
        }
    }
    Ok(counts)
}

struct Response {
    dataset: String,
    item: String,
    question: String,
    text: String,
    criteria: Criteria,
    judgement: String,
    reason: Option<String>,
    revision: u64,
    original: Option<String>,
    note: Option<String>,
}
fn valid_evaluation(j: &str, reason: Option<&str>) -> bool {
    match (j, reason) {
        ("incorrect", Some("no_match" | "unanswered" | "user")) => true,
        ("correct" | "partial" | "ungraded", None) => true,
        _ => false,
    }
}
fn load_response(db: &Connection, exam: &str, position: u32) -> AppResult<Response> {
    let row:Option<(String,String,String,String,String,String)>=db.query_row("SELECT dataset_id,item_id,question_snapshot,response_text,criteria_snapshot_json,type FROM exam_responses WHERE exam_id=?1 AND position=?2",params![exam,position],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let (dataset, item, question, text, criteria, kind) = row.ok_or_else(corrupt)?;
    let criteria: Criteria = decode(&criteria)?;
    if !is_valid_id(&dataset) || !is_valid_id(&item) || is_unanswered(&question) {
        return Err(corrupt());
    }
    match &criteria {
        Criteria::ShortAnswer {
            answer,
            accepted_answers,
        } if kind == "short_answer"
            && is_valid_answer(answer)
            && accepted_answers.iter().all(|a| is_valid_answer(a)) => {}
        Criteria::Descriptive {
            reference_answer,
            key_points,
        } if kind == "descriptive"
            && !is_unanswered(reference_answer)
            && !key_points.is_empty()
            && key_points.iter().all(|p| !is_unanswered(p)) => {}
        _ => return Err(corrupt()),
    }
    let (judgement,reason,revision,latest):(String,Option<String>,u64,Option<String>)=db.query_row("SELECT judgement,incorrect_reason,assessment_revision,latest_event_id FROM response_assessments WHERE exam_id=?1 AND position=?2",params![exam,position],|r|Ok((r.get(0)?,r.get(1)?,sql_revision(r,2)?,r.get(3)?)))?;
    let mut s=db.prepare("SELECT event_id,assessment_revision,previous_judgement,judgement,incorrect_reason,note,method,recorded_at,request_id FROM assessment_events WHERE exam_id=?1 AND position=?2 ORDER BY assessment_revision")?;
    let mut rows = s.query(params![exam, position])?;
    let mut current = "ungraded".to_string();
    let mut n = 0;
    let mut last = None;
    let mut original = None;
    let mut note = None;
    let mut final_reason = None;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let rev: u64 = sql_revision(row, 1)?;
        let previous: Option<String> = row.get(2)?;
        let next: String = row.get(3)?;
        let why: Option<String> = row.get(4)?;
        uuid(&id).map_err(|_| corrupt())?;
        if rev != n + 1
            || (n > 0 && previous.as_deref() != Some(&current))
            || (n == 0 && previous.as_deref().is_some_and(|p| p != "ungraded"))
            || next == "ungraded"
            || !valid_evaluation(&next, why.as_deref())
        {
            return Err(corrupt());
        }
        let method: String = row.get(6)?;
        let recorded: String = row.get(7)?;
        let request: Option<String> = row.get(8)?;
        if chrono::DateTime::parse_from_rfc3339(&recorded).is_err() {
            return Err(corrupt());
        }
        if method == "user" {
            if request.as_deref().is_none_or(|id| uuid(id).is_err())
                || is_unanswered(&text)
                || (n == 0 && kind != "descriptive")
                || (next == "incorrect" && why.as_deref() != Some("user"))
            {
                return Err(corrupt());
            }
        } else {
            if n != 0 || !matches!(method.as_str(), "automatic" | "migration") || request.is_some()
            {
                return Err(corrupt());
            }
            let expected = match &criteria {
                Criteria::ShortAnswer {
                    answer,
                    accepted_answers,
                } => crate::exam::grade(&text, answer, accepted_answers),
                Criteria::Descriptive { .. } if is_unanswered(&text) => {
                    ("incorrect", Some("unanswered"))
                }
                _ => return Err(corrupt()),
            };
            if next != expected.0 || why.as_deref() != expected.1 {
                return Err(corrupt());
            }
        }
        if original.is_none() {
            original = Some(next.clone());
        }
        current = next;
        n = rev;
        last = Some(id);
        note = row.get(5)?;
        final_reason = why;
    }
    if revision > MAX_REV
        || revision != n
        || latest != last
        || judgement != current
        || reason != final_reason
        || !valid_evaluation(&judgement, reason.as_deref())
    {
        return Err(corrupt());
    }
    if n == 0 && (kind != "descriptive" || is_unanswered(&text)) {
        return Err(corrupt());
    }
    Ok(Response {
        dataset,
        item,
        question,
        text,
        criteria,
        judgement,
        reason,
        revision,
        original,
        note,
    })
}

fn retry(
    db: &Connection,
    learner: &str,
    id: &str,
    command: &str,
    hash: &str,
) -> AppResult<Option<MutationResult>> {
    let stored:Option<(String,String,String)>=db.query_row("SELECT command,request_hash,result_json FROM mutation_requests WHERE learner_id=?1 AND request_id=?2",params![learner,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    if let Some((old_command, old_hash, result)) = stored {
        if old_command != command || old_hash != hash {
            return Err(conflict("RequestConflict"));
        }
        let mut result: MutationResult = decode(&result)?;
        result.outcome = "already_applied".into();
        Ok(Some(result))
    } else {
        Ok(None)
    }
}
fn remember(
    db: &Connection,
    learner: &str,
    id: &str,
    command: &str,
    hash: &str,
    result: &MutationResult,
) -> AppResult<()> {
    db.execute(
        "INSERT INTO mutation_requests VALUES (?1,?2,?3,?4,?5)",
        params![learner, id, command, hash, json(result)?],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
