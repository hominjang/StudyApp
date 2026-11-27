use super::*;
use crate::{
    exam::{self, CandidatePreview, CandidateRequest, ExamHistoryRow, PendingExam, RecordEntry},
    validation::is_unanswered,
};

impl Store {
    pub fn preview_candidates(&mut self, request: CandidateRequest) -> AppResult<CandidatePreview> {
        exam::validate_scope(&request.dataset_ids, &request.review_mode)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let (mut total, mut wrong) = (0_u32, 0_u32);
        for id in &request.dataset_ids {
            if !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM datasets WHERE dataset_id=?1)",
                [id],
                |r| r.get::<_, bool>(0),
            )? {
                return Err(AppError::new(
                    "NotFound",
                    "선택한 자료를 찾지 못했어요. 자료 범위를 다시 확인해주세요.",
                    false,
                ));
            }
            let (count,targets):(u32,u32)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(CASE WHEN p.is_review_target=1 THEN 1 ELSE 0 END),0) FROM items i LEFT JOIN item_progress p ON p.learner_id=?1 AND p.dataset_id=i.dataset_id AND p.item_id=i.item_id WHERE i.dataset_id=?2 AND i.is_active=1",params![self.learner_id,id],|r|Ok((r.get(0)?,r.get(1)?)))?;
            total = total
                .checked_add(count)
                .ok_or_else(|| AppError::validation("question_count"))?;
            wrong = wrong
                .checked_add(targets)
                .ok_or_else(|| AppError::validation("question_count"))?;
        }
        let result = CandidatePreview {
            candidate_count: if request.review_mode == "wrong_only" {
                wrong
            } else {
                total
            },
            total_count: total,
            review_target_count: wrong,
        };
        tx.commit()?;
        Ok(result)
    }

    pub fn exam_history(&mut self, filter: &str) -> AppResult<Vec<RecordEntry<ExamHistoryRow>>> {
        exam_history(&mut self.connection, &self.learner_id, 1, filter)
    }
    pub fn pending_exams(&mut self) -> AppResult<Vec<RecordEntry<PendingExam>>> {
        pending_exams(&mut self.connection, &self.learner_id, 1)
    }
}

pub(crate) fn exam_history(
    connection: &mut Connection,
    learner: &str,
    version: u8,
    filter: &str,
) -> AppResult<Vec<RecordEntry<ExamHistoryRow>>> {
    if !matches!(
        filter,
        "all" | "in_progress" | "submitted" | "completed" | "assessment_pending"
    ) {
        return Err(AppError::validation("filter"));
    }
    let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let mut result = read_rows(&tx, learner, filter, version)?;
    if filter == "assessment_pending" {
        result.retain(|entry| {
            entry
                .record()
                .is_some_and(|r| r.assessment_status.as_deref() == Some("pending"))
        });
    }
    result.sort_by(|a, b| match (a.record(), b.record()) {
        (Some(a), Some(b)) => (a.status == "completed")
            .cmp(&(b.status == "completed"))
            .then_with(|| {
                if a.status == "completed" && b.status == "completed" {
                    b.completion_order.cmp(&a.completion_order)
                } else {
                    time(&b.saved_at).cmp(&time(&a.saved_at))
                }
            })
            .then_with(|| a.exam_id.cmp(&b.exam_id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    tx.commit()?;
    Ok(result)
}

pub fn pending_exams(
    connection: &mut Connection,
    learner: &str,
    version: u8,
) -> AppResult<Vec<RecordEntry<PendingExam>>> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let mut rows = read_rows(&tx, learner, "pending", version)?;
    rows.sort_by(|a, b| match (a.record(), b.record()) {
        (Some(a), Some(b)) => time(&b.saved_at)
            .cmp(&time(&a.saved_at))
            .then_with(|| a.exam_id.cmp(&b.exam_id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    let result = rows
        .into_iter()
        .map(|entry| match entry {
            RecordEntry::Error { row_key, error } => RecordEntry::Error { row_key, error },
            RecordEntry::Exam { row_key, record: r } => RecordEntry::Exam {
                row_key,
                record: PendingExam {
                    exam_id: r.exam_id,
                    status: r.status,
                    dataset_titles: r.dataset_titles,
                    question_count: r.question_count,
                    answered_count: r.answered_count,
                    current_position: r.current_position,
                    saved_at: r.saved_at,
                    error: None,
                },
            },
        })
        .collect();
    tx.commit()?;
    Ok(result)
}
fn time(text: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(text).ok()
}

fn read_rows(
    db: &Connection,
    learner: &str,
    filter: &str,
    version: u8,
) -> AppResult<Vec<RecordEntry<ExamHistoryRow>>> {
    let mut statement=db.prepare("SELECT rowid,exam_id,status,saved_at,completed_at,completion_order FROM exams WHERE learner_id=?1 AND (?2='all' OR (?2='pending' AND status IN ('in_progress','submitted')) OR status=?2 OR (?2='assessment_pending' AND status='completed')) ORDER BY rowid")?;
    let mut rows = statement.query(params![learner, filter])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let row_key = format!("record-{}", row.get::<_, i64>(0)?);
        let read = || -> AppResult<ExamHistoryRow> {
            let id: String = row.get(1)?;
            let status: String = row.get(2)?;
            let saved: String = row.get(3)?;
            let completed: Option<String> = row.get(4)?;
            let order: Option<i64> = row.get(5)?;
            if time(&saved).is_none()
                || completed.as_ref().is_some_and(|t| time(t).is_none())
                || !matches!(
                    (status.as_str(), completed.is_some(), order),
                    ("in_progress" | "submitted", false, None) | ("completed", true, Some(1..))
                )
            {
                return Err(AppError::inconsistent());
            }
            history_row(db, learner, &id, &saved, &completed, order, version)
        };
        result.push(match read() {
            Ok(record) => RecordEntry::Exam { row_key, record },
            Err(_) => RecordEntry::Error {
                row_key,
                error: "저장된 기록의 일부를 읽지 못했어요.".into(),
            },
        });
    }
    Ok(result)
}

fn history_row(
    tx: &Connection,
    learner_id: &str,
    id: &str,
    saved_at: &str,
    completed_at: &Option<String>,
    completion_order: Option<i64>,
    version: u8,
) -> AppResult<ExamHistoryRow> {
    let saved = if version == 2 {
        crate::assessment::v2_session(tx, learner_id, id)?
    } else {
        super::exams::session(tx, learner_id, id)?
    };
    let counts = if version == 2 && saved.status == "completed" {
        Some(crate::assessment::history_counts(tx, learner_id, id)?)
    } else {
        None
    };
    let correct_count = if let Some(counts) = &counts {
        Some(counts.correct as usize)
    } else if saved.status == "completed" {
        let (correct,graded):(u32,u32)=tx.query_row("SELECT COALESCE(SUM(judgement='correct'),0),COALESCE(SUM(judgement IN ('correct','incorrect')),0) FROM exam_responses WHERE exam_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if graded as usize != saved.items.len()
            || completed_at.is_none()
            || completion_order.is_none_or(|n| n < 1)
        {
            return Err(AppError::inconsistent());
        }
        Some(correct as usize)
    } else {
        None
    };
    Ok(ExamHistoryRow {
        assessment_status: counts.as_ref().map(|c| {
            if c.ungraded > 0 {
                "pending"
            } else {
                "complete"
            }
            .into()
        }),
        assessment_counts: counts,
        exam_id: id.into(),
        status: saved.status,
        dataset_titles: saved.dataset_titles,
        question_count: saved.items.len(),
        answered_count: saved
            .items
            .iter()
            .filter(|i| !is_unanswered(&i.response_text))
            .count(),
        current_position: saved.current_position,
        saved_at: saved_at.into(),
        completed_at: completed_at.clone(),
        completion_order,
        correct_count,
        error: None,
    })
}

#[cfg(test)]
mod tests;
