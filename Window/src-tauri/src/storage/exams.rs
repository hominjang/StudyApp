use super::*;
use crate::exam::{
    self, ExamResult, ExamSession, ResultItem, SaveProgressResult, SessionItem, StartExamRequest,
    SubmitExamRequest,
};
use crate::validation::{is_unanswered, is_valid_answer};

fn decode<T: serde::de::DeserializeOwned>(text: &str) -> AppResult<T> {
    serde_json::from_str(text).map_err(|_| AppError::inconsistent())
}

#[cfg(test)]
mod progress_tests;
#[cfg(test)]
mod tests;
fn encode<T: serde::Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|_| AppError::inconsistent())
}

pub(super) fn session(db: &Connection, learner: &str, id: &str) -> AppResult<ExamSession> {
    exam::validate_exam_id(id)?;
    let row = db.query_row("SELECT learner_id,status,revision,current_position,dataset_ids_json,dataset_titles_snapshot_json,question_count,count_mode,order_mode,review_mode FROM exams WHERE exam_id=?1", [id], |r| {
        Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,u32>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,u32>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?,r.get::<_,String>(9)?))
    }).optional()?.ok_or_else(|| AppError::new("NotFound", "시험을 찾지 못했어요.", false))?;
    if row.0 != learner {
        return Err(AppError::new(
            "OwnershipMismatch",
            "이 학습 공간의 시험이 아니에요.",
            false,
        ));
    }
    let mut statement = db.prepare("SELECT position,dataset_id,item_id,question_snapshot,response_text FROM exam_responses WHERE exam_id=?1 ORDER BY position")?;
    let items = statement
        .query_map([id], |r| {
            Ok(SessionItem {
                item_type: "short_answer".into(),
                position: r.get(0)?,
                dataset_id: r.get(1)?,
                item_id: r.get(2)?,
                question: r.get(3)?,
                response_text: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let dataset_ids: Vec<String> = decode(&row.4)?;
    let dataset_titles: Vec<String> = decode(&row.5)?;
    if row.2 < 0
        || row.2 > 9_007_199_254_740_991
        || items.len() != row.6 as usize
        || items.is_empty()
        || row.3 >= row.6
        || dataset_ids.len() != dataset_titles.len()
        || items.iter().enumerate().any(|(index, item)| {
            item.position as usize != index || !dataset_ids.contains(&item.dataset_id)
        })
    {
        return Err(AppError::inconsistent());
    }
    Ok(ExamSession {
        db_version: 1,
        exam_id: id.into(),
        status: row.1,
        revision: row.2 as u64,
        current_position: row.3,
        settings: StartExamRequest {
            exam_id: id.into(),
            dataset_ids,
            count_mode: row.7.clone(),
            count: if row.7 == "count" { Some(row.6) } else { None },
            order_mode: row.8,
            review_mode: row.9,
        },
        dataset_titles,
        items,
    })
}

impl Store {
    pub fn save_progress(&mut self, request: SubmitExamRequest) -> AppResult<SaveProgressResult> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = session(&tx, &self.learner_id, &request.exam_id)?;
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
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let result = session(&tx, &self.learner_id, id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn start_exam(&mut self, request: StartExamRequest) -> AppResult<ExamSession> {
        request.validate()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM exams WHERE exam_id=?1)",
            [&request.exam_id],
            |r| r.get::<_, bool>(0),
        )? {
            let existing = session(&tx, &self.learner_id, &request.exam_id)?;
            if existing.settings != request {
                return Err(AppError::new(
                    "RevisionConflict",
                    "같은 시험 ID에 다른 설정이 있어요. 설정을 확인해주세요.",
                    false,
                ));
            }
            tx.commit()?;
            return Ok(existing);
        }
        let mut titles = Vec::new();
        let mut candidates = Vec::new();
        for dataset_id in &request.dataset_ids {
            let title: String = tx
                .query_row(
                    "SELECT title FROM datasets WHERE dataset_id=?1",
                    [dataset_id],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(|| {
                    AppError::new(
                        "NotFound",
                        "선택한 자료를 찾지 못했어요. 자료 목록을 다시 확인해주세요.",
                        false,
                    )
                })?;
            titles.push(title);
            let mut statement=tx.prepare("SELECT i.item_id,i.question,i.answer,i.accepted_answers_json,COALESCE(p.is_review_target,0) FROM items i LEFT JOIN item_progress p ON p.learner_id=?2 AND p.dataset_id=i.dataset_id AND p.item_id=i.item_id WHERE i.dataset_id=?1 AND i.is_active=1 ORDER BY i.position,i.item_id")?;
            let rows = statement
                .query_map(params![dataset_id, self.learner_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, bool>(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (item_id, question, answer, aliases_json, target) in rows {
                let aliases: Vec<String> = decode(&aliases_json)?;
                if is_unanswered(&question)
                    || !is_valid_answer(&answer)
                    || aliases.iter().any(|v| !is_valid_answer(v))
                {
                    return Err(AppError::inconsistent());
                }
                candidates.push((
                    dataset_id.clone(),
                    item_id,
                    question,
                    answer,
                    aliases_json,
                    target,
                ));
            }
        }
        if candidates.len() > u32::MAX as usize {
            return Err(AppError::validation("question_count"));
        }
        let targets: Vec<bool> = candidates.iter().map(|item| item.5).collect();
        let selected = exam::select_indices(
            &targets,
            &request.review_mode,
            &request.order_mode,
            request.count,
            &mut |indices| exam::shuffle(indices),
        )?;
        let count = selected.len();
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO exams(exam_id,learner_id,status,revision,dataset_ids_json,dataset_titles_snapshot_json,question_count,count_mode,order_mode,review_mode,current_position,grading_rule_version,started_at,saved_at) VALUES (?1,?2,'in_progress',0,?3,?4,?5,?6,?7,?8,0,?9,?10,?10)",
            params![request.exam_id,self.learner_id,encode(&request.dataset_ids)?,encode(&titles)?,count as i64,request.count_mode,request.order_mode,request.review_mode,exam::RULE,now])?;
        for (position, index) in selected.into_iter().enumerate() {
            let (dataset, item, question, answer, aliases, _) = &candidates[index];
            tx.execute("INSERT INTO exam_responses(exam_id,position,dataset_id,item_id,question_snapshot,answer_snapshot,accepted_answers_snapshot_json,response_text,judgement) VALUES (?1,?2,?3,?4,?5,?6,?7,'','ungraded')",params![request.exam_id,position as i64,dataset,item,question,answer,aliases])?;
        }
        let result = session(&tx, &self.learner_id, &request.exam_id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn submit_exam(&mut self, request: SubmitExamRequest) -> AppResult<ExamSession> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = session(&tx, &self.learner_id, &request.exam_id)?;
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
        let result = session(&tx, &self.learner_id, &request.exam_id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn complete_exam(&mut self, id: &str) -> AppResult<ExamSession> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = session(&tx, &self.learner_id, id)?;
        if current.status == "completed" {
            tx.commit()?;
            return Ok(current);
        }
        if current.status != "submitted" {
            return Err(AppError::new(
                "InvalidState",
                "먼저 시험의 답변을 최종 제출해주세요.",
                false,
            ));
        }
        let rule: String = tx.query_row(
            "SELECT grading_rule_version FROM exams WHERE exam_id=?1",
            [id],
            |r| r.get(0),
        )?;
        if rule != exam::RULE {
            return Err(AppError::new(
                "UnsupportedFormat",
                "이 시험의 채점 규칙을 지원하지 않아요. 제출 답변은 보관되어 있어요.",
                false,
            ));
        }
        let snapshots = {
            let mut statement=tx.prepare("SELECT position,response_text,answer_snapshot,accepted_answers_snapshot_json FROM exam_responses WHERE exam_id=?1 ORDER BY position")?;
            let rows = statement
                .query_map([id], |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for (position, response, answer, aliases) in snapshots {
            let aliases: Vec<String> = decode(&aliases)?;
            if !is_valid_answer(&answer) || aliases.iter().any(|v| !is_valid_answer(v)) {
                return Err(AppError::inconsistent());
            }
            let (judgement, reason) = exam::grade(&response, &answer, &aliases);
            tx.execute("UPDATE exam_responses SET judgement=?1,incorrect_reason=?2 WHERE exam_id=?3 AND position=?4",params![judgement,reason,id,position])?;
        }
        let order: i64 = tx.query_row(
            "SELECT COALESCE(MAX(completion_order),0)+1 FROM exams WHERE learner_id=?1",
            [&self.learner_id],
            |r| r.get(0),
        )?;
        let now = Utc::now().to_rfc3339();
        tx.execute("UPDATE exams SET status='completed',completed_at=?1,saved_at=?1,completion_order=?2 WHERE exam_id=?3",params![now,order,id])?;
        for item in &current.items {
            let history = {
                let mut statement=tx.prepare("SELECT r.judgement,e.exam_id FROM exam_responses r JOIN exams e ON e.exam_id=r.exam_id WHERE e.learner_id=?1 AND e.status='completed' AND r.dataset_id=?2 AND r.item_id=?3 ORDER BY e.completion_order")?;
                let rows = statement
                    .query_map(
                        params![self.learner_id, item.dataset_id, item.item_id],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                rows
            };
            let (mut target, mut streak) = (false, 0);
            for (judgement, _) in &history {
                match judgement.as_str() {
                    "incorrect" => {
                        target = true;
                        streak = 0;
                    }
                    "correct" if target => {
                        streak += 1;
                        if streak == 3 {
                            target = false;
                        }
                    }
                    "correct" => {}
                    _ => return Err(AppError::inconsistent()),
                }
            }
            let (last_result, last_exam) = history.last().ok_or_else(AppError::inconsistent)?;
            tx.execute("INSERT INTO item_progress(learner_id,dataset_id,item_id,is_review_target,correct_streak,last_result,last_completed_exam_id) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(learner_id,dataset_id,item_id) DO UPDATE SET is_review_target=excluded.is_review_target,correct_streak=excluded.correct_streak,last_result=excluded.last_result,last_completed_exam_id=excluded.last_completed_exam_id",params![self.learner_id,item.dataset_id,item.item_id,target,streak,last_result,last_exam])?;
        }
        let result = session(&tx, &self.learner_id, id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn exam_result(&mut self, id: &str) -> AppResult<ExamResult> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let current = session(&tx, &self.learner_id, id)?;
        if current.status != "completed" {
            return Err(AppError::new(
                "InvalidState",
                "모든 답변을 제출하고 채점이 끝난 뒤 결과를 확인할 수 있어요.",
                false,
            ));
        }
        let (completed_at, completion_order): (String, i64) = tx.query_row(
            "SELECT completed_at,completion_order FROM exams WHERE exam_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut items = Vec::new();
        for item in current.items {
            let (answer,aliases,judgement,reason):(String,String,String,Option<String>)=tx.query_row("SELECT answer_snapshot,accepted_answers_snapshot_json,judgement,incorrect_reason FROM exam_responses WHERE exam_id=?1 AND position=?2",params![id,item.position],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
            if !matches!(judgement.as_str(), "correct" | "incorrect") {
                return Err(AppError::inconsistent());
            }
            let (target,streak):(bool,u8)=tx.query_row("SELECT is_review_target,correct_streak FROM item_progress WHERE learner_id=?1 AND dataset_id=?2 AND item_id=?3",params![self.learner_id,item.dataset_id,item.item_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let supplement: Option<String> = tx
                .query_row(
                    "SELECT payload_json FROM supplements WHERE dataset_id=?1 AND item_id=?2",
                    params![item.dataset_id, item.item_id],
                    |r| r.get(0),
                )
                .optional()?;
            items.push(ResultItem {
                position: item.position,
                question: item.question,
                response_text: item.response_text,
                answer,
                accepted_answers: decode(&aliases)?,
                judgement,
                incorrect_reason: reason,
                is_review_target: target,
                correct_streak: streak,
                supplement: supplement.as_deref().map(decode).transpose()?,
            });
        }
        let correct_count = items.iter().filter(|i| i.judgement == "correct").count();
        let unanswered_count = items
            .iter()
            .filter(|i| i.incorrect_reason.as_deref() == Some("unanswered"))
            .count();
        let result = ExamResult {
            exam_id: id.into(),
            completed_at,
            completion_order,
            settings: current.settings,
            dataset_titles: current.dataset_titles,
            correct_count,
            incorrect_count: items.len() - correct_count,
            unanswered_count,
            items,
        };
        tx.commit()?;
        Ok(result)
    }
}
