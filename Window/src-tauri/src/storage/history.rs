use super::*;
use crate::{
    exam::{self, CandidatePreview, CandidateRequest, ExamHistoryRow},
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

    pub fn exam_history(&mut self, filter: &str) -> AppResult<Vec<ExamHistoryRow>> {
        if !matches!(filter, "all" | "in_progress" | "submitted" | "completed") {
            return Err(AppError::validation("filter"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let rows = {
            let mut statement=tx.prepare("SELECT exam_id,status,saved_at,completed_at,completion_order FROM exams WHERE learner_id=?1 AND (?2='all' OR status=?2) ORDER BY CASE WHEN status='completed' THEN 1 ELSE 0 END,CASE WHEN status!='completed' THEN saved_at END DESC,CASE WHEN status='completed' THEN completion_order END DESC,exam_id")?;
            let rows = statement
                .query_map(params![self.learner_id, filter], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut result = Vec::new();
        for (id, status, saved_at, completed_at, completion_order) in rows {
            // 손상된 기록 하나가 목록 전체를 막지 않도록 해당 행에만 오류를 표시한다.
            let row = history_row(&tx, &self.learner_id, &id, &saved_at, &completed_at, completion_order)
                .unwrap_or_else(|error| ExamHistoryRow {
                    exam_id: id.clone(),
                    status,
                    dataset_titles: Vec::new(),
                    question_count: 0,
                    answered_count: 0,
                    current_position: 0,
                    saved_at: saved_at.clone(),
                    completed_at: completed_at.clone(),
                    completion_order,
                    correct_count: None,
                    error: Some(error.message),
                });
            result.push(row);
        }
        tx.commit()?;
        Ok(result)
    }
}

fn history_row(
    tx: &Connection,
    learner_id: &str,
    id: &str,
    saved_at: &str,
    completed_at: &Option<String>,
    completion_order: Option<i64>,
) -> AppResult<ExamHistoryRow> {
    let saved = super::exams::session(tx, learner_id, id)?;
    let correct_count = if saved.status == "completed" {
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
