use super::*;
use crate::{
    exam::{
        self, CandidatePreview, CandidateRequest, ExamHistoryRow, ExamResult, ExamSession,
        PendingExam, RecordEntry, SaveProgressResult, SessionItem, StartExamRequest,
        SubmitExamRequest,
    },
    storage::{AppStatus, DatasetSummary},
};
use std::collections::HashMap;

pub(crate) fn session(db: &Connection, learner: &str, id: &str) -> AppResult<ExamSession> {
    uuid(id)?;
    let row:Option<(String,String,u64,u32,String,String,u32,String,String,String)>=db.query_row("SELECT learner_id,status,revision,current_position,dataset_ids_json,dataset_titles_snapshot_json,question_count,count_mode,order_mode,review_mode FROM exams WHERE exam_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,sql_revision(r,2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional()?;
    let (owner, status, revision, current, scope, titles, count, mode, order, review) =
        row.ok_or_else(|| AppError::new("NotFound", "시험을 찾지 못했어요.", false))?;
    if owner != learner {
        return Err(AppError::new(
            "OwnershipMismatch",
            "이 학습 공간의 시험이 아니에요.",
            false,
        ));
    }
    let settings = StartExamRequest {
        exam_id: id.into(),
        dataset_ids: decode(&scope)?,
        count_mode: mode.clone(),
        count: if mode == "count" { Some(count) } else { None },
        order_mode: order,
        review_mode: review,
    };
    settings.validate()?;
    let dataset_titles: Vec<String> = decode(&titles)?;
    let items:Vec<SessionItem>=db.prepare("SELECT position,dataset_id,item_id,question_snapshot,response_text,type FROM exam_responses WHERE exam_id=?1 ORDER BY position")?.query_map([id],|r|Ok(SessionItem{position:r.get(0)?,dataset_id:r.get(1)?,item_id:r.get(2)?,question:r.get(3)?,response_text:r.get(4)?,item_type:r.get(5)?}))?.collect::<Result<_,_>>()?;
    if !matches!(status.as_str(), "in_progress" | "submitted" | "completed")
        || count == 0
        || current >= count
        || items.len() != count as usize
        || dataset_titles.len() != settings.dataset_ids.len()
        || items.iter().enumerate().any(|(i, item)| {
            item.position as usize != i
                || !settings.dataset_ids.contains(&item.dataset_id)
                || !is_valid_id(&item.item_id)
                || is_unanswered(&item.question)
                || !matches!(item.item_type.as_str(), "short_answer" | "descriptive")
        })
    {
        return Err(corrupt());
    }
    Ok(ExamSession {
        db_version: 2,
        exam_id: id.into(),
        status,
        revision,
        current_position: current,
        settings,
        dataset_titles,
        items,
    })
}

fn criterion(text: &str, kind: &str) -> AppResult<Criteria> {
    let value: Criteria = decode(text)?;
    match &value {
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
    Ok(value)
}

impl AssessmentStore {
    pub fn list_datasets(&mut self) -> AppResult<Vec<DatasetSummary>> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let rows=tx.prepare("SELECT d.dataset_id,d.title,d.position,COUNT(i.item_id),(SELECT COUNT(*) FROM item_progress p JOIN items pi ON pi.dataset_id=p.dataset_id AND pi.item_id=p.item_id WHERE p.learner_id=?1 AND p.dataset_id=d.dataset_id AND p.is_review_target=1 AND pi.is_active=1) FROM datasets d LEFT JOIN items i ON i.dataset_id=d.dataset_id AND i.is_active=1 GROUP BY d.dataset_id ORDER BY d.position,d.dataset_id")?.query_map([&self.learner],|r|Ok(DatasetSummary{dataset_id:r.get(0)?,title:r.get(1)?,position:r.get::<_,u32>(2)? as usize,question_count:r.get::<_,u32>(3)? as usize,review_target_count:r.get::<_,u32>(4)? as usize}))?.collect::<Result<Vec<_>,_>>()?;
        tx.commit()?;
        Ok(rows)
    }
    pub fn status(&mut self) -> AppResult<AppStatus> {
        let datasets = self.list_datasets()?;
        Ok(AppStatus {
            runtime: "desktop",
            storage: "ready",
            dataset_count: datasets.len(),
            question_count: datasets.iter().map(|d| d.question_count).sum(),
        })
    }
    pub fn preview_candidates(&mut self, r: CandidateRequest) -> AppResult<CandidatePreview> {
        exam::validate_scope(&r.dataset_ids, &r.review_mode)?;
        let mut total = 0u32;
        let mut targets = 0u32;
        for id in r.dataset_ids {
            let exists: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM datasets WHERE dataset_id=?1)",
                [&id],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(AppError::new(
                    "NotFound",
                    "선택한 자료를 찾지 못했어요.",
                    false,
                ));
            }
            let (n,t):(u32,u32)=self.db.query_row("SELECT COUNT(*),COALESCE(SUM(COALESCE(p.is_review_target,0)),0) FROM items i LEFT JOIN item_progress p ON p.learner_id=?1 AND p.dataset_id=i.dataset_id AND p.item_id=i.item_id WHERE i.dataset_id=?2 AND i.is_active=1",params![self.learner,id],|r|Ok((r.get(0)?,r.get(1)?)))?;
            total = total.checked_add(n).ok_or_else(corrupt)?;
            targets = targets.checked_add(t).ok_or_else(corrupt)?;
        }
        Ok(CandidatePreview {
            candidate_count: if r.review_mode == "wrong_only" {
                targets
            } else {
                total
            },
            total_count: total,
            review_target_count: targets,
        })
    }
    pub fn exam_history(&mut self, filter: &str) -> AppResult<Vec<RecordEntry<ExamHistoryRow>>> {
        crate::storage::history::exam_history(&mut self.db, &self.learner, 2, filter)
    }
    pub fn pending_exams(&mut self) -> AppResult<Vec<RecordEntry<PendingExam>>> {
        crate::storage::history::pending_exams(&mut self.db, &self.learner, 2)
    }
    pub fn exam_result(&mut self, _id: &str) -> AppResult<ExamResult> {
        Err(AppError::new(
            "UnsupportedFormat",
            "확장 결과 조회로 평가를 확인해주세요.",
            false,
        ))
    }

    pub fn start_exam(&mut self, request: StartExamRequest) -> AppResult<ExamSession> {
        request.validate()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM exams WHERE exam_id=?1)",
            [&request.exam_id],
            |r| r.get::<_, bool>(0),
        )? {
            let existing = session(&tx, &self.learner, &request.exam_id)?;
            if existing.settings != request {
                return Err(conflict("RevisionConflict"));
            }
            tx.commit()?;
            return Ok(existing);
        }
        let mut titles = Vec::new();
        let mut candidates = Vec::new();
        for dataset in &request.dataset_ids {
            let title: Option<String> = tx
                .query_row(
                    "SELECT title FROM datasets WHERE dataset_id=?1",
                    [dataset],
                    |r| r.get(0),
                )
                .optional()?;
            titles.push(
                title.ok_or_else(|| {
                    AppError::new("NotFound", "선택한 자료를 찾지 못했어요.", false)
                })?,
            );
            let mut statement=tx.prepare("SELECT i.item_id,i.question,i.type,i.criteria_json,COALESCE(p.is_review_target,0) FROM items i LEFT JOIN item_progress p ON p.learner_id=?1 AND p.dataset_id=i.dataset_id AND p.item_id=i.item_id WHERE i.dataset_id=?2 AND i.is_active=1 ORDER BY i.position,i.item_id")?;
            let mut rows = statement.query(params![self.learner, dataset])?;
            while let Some(row) = rows.next()? {
                let (id, q, kind, c, target): (String, String, String, String, bool) = (
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                );
                if !is_valid_id(&id) || is_unanswered(&q) {
                    return Err(corrupt());
                }
                criterion(&c, &kind)?;
                if request.review_mode != "general" {
                    checked_progress(&tx, &self.learner, dataset, &id)?;
                }
                candidates.push((dataset.clone(), id, q, kind, c, target));
            }
        }
        let targets: Vec<_> = candidates.iter().map(|c| c.5).collect();
        let selected = exam::select_indices(
            &targets,
            &request.review_mode,
            &request.order_mode,
            request.count,
            &mut |indices| exam::shuffle(indices),
        )?;
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO exams(exam_id,learner_id,status,revision,dataset_ids_json,dataset_titles_snapshot_json,question_count,count_mode,order_mode,review_mode,current_position,grading_rule_version,started_at,saved_at) VALUES (?1,?2,'in_progress',0,?3,?4,?5,?6,?7,?8,0,'mixed-v2',?9,?9)",params![request.exam_id,self.learner,json(&request.dataset_ids)?,json(&titles)?,selected.len() as i64,request.count_mode,request.order_mode,request.review_mode,now])?;
        for (position, index) in selected.into_iter().enumerate() {
            let (d, id, q, kind, c, _) = &candidates[index];
            checked_progress(&tx, &self.learner, d, id)?;
            tx.execute(
                "INSERT INTO exam_responses VALUES (?1,?2,?3,?4,?5,?6,?7,'')",
                params![request.exam_id, position as i64, d, id, kind, q, c],
            )?;
            tx.execute(
                "INSERT INTO response_assessments VALUES (?1,?2,'ungraded',NULL,0,NULL)",
                params![request.exam_id, position as i64],
            )?;
        }
        let result = session(&tx, &self.learner, &request.exam_id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn complete_exam(&mut self, id: &str) -> AppResult<ExamSession> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = session(&tx, &self.learner, id)?;
        if current.status == "completed" {
            tx.commit()?;
            return Ok(current);
        }
        if current.status != "submitted" {
            return Err(AppError::new(
                "InvalidState",
                "먼저 답변을 제출해주세요.",
                false,
            ));
        }
        let rule: String = tx.query_row(
            "SELECT grading_rule_version FROM exams WHERE exam_id=?1",
            [id],
            |r| r.get(0),
        )?;
        if !matches!(rule.as_str(), "mixed-v2" | "exact-v1") {
            return Err(AppError::new(
                "UnsupportedFormat",
                "채점 규칙을 지원하지 않아요. 제출 답변은 보관돼요.",
                false,
            ));
        }
        let mut before = HashMap::new();
        for item in &current.items {
            before.insert(
                item.position,
                checked_progress(&tx, &self.learner, &item.dataset_id, &item.item_id)?.revision,
            );
            let c:String=tx.query_row("SELECT criteria_snapshot_json FROM exam_responses WHERE exam_id=?1 AND position=?2",params![id,item.position],|r|r.get(0))?;
            let criteria = criterion(&c, &item.item_type)?;
            let initial:(String,u64,Option<String>)=tx.query_row("SELECT judgement,assessment_revision,latest_event_id FROM response_assessments WHERE exam_id=?1 AND position=?2",params![id,item.position],|r|Ok((r.get(0)?,sql_revision(r,1)?,r.get(2)?)))?;
            if initial != ("ungraded".into(), 0, None) {
                return Err(corrupt());
            }
            let result = match &criteria {
                Criteria::ShortAnswer {
                    answer,
                    accepted_answers,
                } => Some(exam::grade(&item.response_text, answer, accepted_answers)),
                Criteria::Descriptive { .. } if is_unanswered(&item.response_text) => {
                    Some(("incorrect", Some("unanswered")))
                }
                Criteria::Descriptive { .. } => None,
            };
            if let Some((judgement, reason)) = result {
                let event = Uuid::new_v4().to_string();
                tx.execute("INSERT INTO assessment_events VALUES (?1,?2,?3,1,NULL,?4,?5,'automatic',?6,NULL,NULL)",params![event,id,item.position,judgement,reason,Utc::now().to_rfc3339()])?;
                tx.execute("UPDATE response_assessments SET judgement=?1,incorrect_reason=?2,assessment_revision=1,latest_event_id=?3 WHERE exam_id=?4 AND position=?5",params![judgement,reason,event,id,item.position])?;
            }
        }
        let order: i64 = tx.query_row(
            "SELECT COALESCE(MAX(completion_order),0)+1 FROM exams WHERE learner_id=?1",
            [&self.learner],
            |r| r.get(0),
        )?;
        let seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(event_seq),0)+1 FROM learning_events WHERE learner_id=?1",
            [&self.learner],
            |r| r.get(0),
        )?;
        if order < 1 || seq < 1 || order as u64 > MAX_REV || seq as u64 > MAX_REV {
            return Err(corrupt());
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("UPDATE exams SET status='completed',completed_at=?1,saved_at=?1,completion_order=?2 WHERE exam_id=?3",params![now,order,id])?;
        tx.execute(
            "INSERT INTO learning_events VALUES (?1,?2,?3,'exam_attempt',?4,NULL,NULL,?5)",
            params![self.learner, seq, Uuid::new_v4().to_string(), id, now],
        )?;
        for item in &current.items {
            let p = compute_progress(&tx, &self.learner, &item.dataset_id, &item.item_id)?;
            let revision = before[&item.position];
            if revision > 0 || p.target || p.last.is_some() {
                save_progress(
                    &tx,
                    &self.learner,
                    &item.dataset_id,
                    &item.item_id,
                    p,
                    revision,
                )?;
            }
        }
        let result = session(&tx, &self.learner, id)?;
        tx.commit()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;

impl AssessmentStore {
    pub fn save_progress(&mut self, request: SubmitExamRequest) -> AppResult<SaveProgressResult> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = session(&tx, &self.learner, &request.exam_id)?;
        let response = |outcome, revision| SaveProgressResult {
            outcome,
            revision,
            status: current.status.clone(),
        };
        if current.status != "in_progress" {
            tx.commit()?;
            return Ok(response("locked", current.revision));
        }
        request.validate(current.items.len())?;
        if request.revision < current.revision {
            tx.commit()?;
            return Ok(response("stale", current.revision));
        }
        if request.revision == current.revision {
            if request.current_position != current.current_position
                || request
                    .responses
                    .iter()
                    .zip(&current.items)
                    .any(|(text, item)| text != &item.response_text)
            {
                return Err(AppError::new("RevisionConflict","같은 저장 버전에 다른 답변이 있어요. 입력을 유지한 채 저장 상태를 확인해주세요.",false));
            }
            tx.commit()?;
            return Ok(response("already_saved", current.revision));
        }
        for (position, text) in request.responses.iter().enumerate() {
            tx.execute(
                "UPDATE exam_responses SET response_text=?1 WHERE exam_id=?2 AND position=?3",
                params![text, request.exam_id, position as i64],
            )?;
        }
        tx.execute(
            "UPDATE exams SET revision=?1,current_position=?2,saved_at=?3 WHERE exam_id=?4",
            params![
                request.revision as i64,
                request.current_position,
                Utc::now().to_rfc3339(),
                request.exam_id
            ],
        )?;
        tx.commit()?;
        Ok(response("saved", request.revision))
    }

    pub fn exam_session(&mut self, id: &str) -> AppResult<ExamSession> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let result = session(&tx, &self.learner, id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn submit_exam(&mut self, request: SubmitExamRequest) -> AppResult<ExamSession> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = session(&tx, &self.learner, &request.exam_id)?;
        if current.status != "in_progress" {
            tx.commit()?;
            return Ok(current);
        }
        request.validate(current.items.len())?;
        if request.revision < current.revision
            || (request.revision == current.revision
                && (request.current_position != current.current_position
                    || request
                        .responses
                        .iter()
                        .zip(&current.items)
                        .any(|(text, item)| text != &item.response_text)))
        {
            return Err(AppError::new(
                "RevisionConflict",
                "저장된 답변과 제출 버전이 달라요. 현재 입력을 유지한 채 상태를 확인해주세요.",
                false,
            ));
        }
        for (position, text) in request.responses.iter().enumerate() {
            tx.execute(
                "UPDATE exam_responses SET response_text=?1 WHERE exam_id=?2 AND position=?3",
                params![text, request.exam_id, position as i64],
            )?;
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("UPDATE exams SET status='submitted',revision=?1,current_position=?2,submitted_at=?3,saved_at=?3 WHERE exam_id=?4",params![request.revision as i64,request.current_position,now,request.exam_id])?;
        let result = session(&tx, &self.learner, &request.exam_id)?;
        tx.commit()?;
        Ok(result)
    }
}
