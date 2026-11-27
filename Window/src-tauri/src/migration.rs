//! Prepares a new v2 copy; never activates or mutates the original v1 store.
use crate::{
    error::{AppError, AppResult},
    exam::{grade, validate_exam_id, StartExamRequest},
    learning_format::Criteria,
    validation::{is_unanswered, is_valid_answer, is_valid_id},
};
use rusqlite::{
    backup::{Backup, StepResult},
    params, Connection, OpenFlags, OptionalExtension, TransactionBehavior,
};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    time::{Duration, Instant},
};
use uuid::Uuid;

const V2: &str = include_str!("../migrations/V002_Extension.sql");
#[derive(Debug, PartialEq, Eq)]
pub struct MigrationReport {
    pub item_count: usize,
    pub response_count: usize,
    pub assessment_count: usize,
    pub attempt_count: usize,
}

fn metadata(db: &Connection, key: &str) -> AppResult<Option<String>> {
    Ok(db
        .query_row("SELECT value FROM app_metadata WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()?)
}
fn integrity(db: &Connection) -> AppResult<()> {
    let checks = db
        .prepare("PRAGMA integrity_check")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if checks != ["ok"] || db.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(AppError::inconsistent());
    }
    Ok(())
}
fn io_error() -> AppError {
    AppError::new(
        "MigrationFileFailure",
        "전환 사본을 만들지 못했어요. 원본은 유지됩니다.",
        true,
    )
}

/// Destination must be new. Successful output is a candidate, not an active app DB.
pub fn prepare_v2_copy(source: &Path, destination: &Path) -> AppResult<MigrationReport> {
    if destination.exists() {
        return Err(AppError::new(
            "DestinationExists",
            "전환 사본 경로가 이미 있어요. 기존 파일은 유지됩니다.",
            false,
        ));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let original = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    original.busy_timeout(Duration::from_secs(3))?;
    let temporary = tempfile::Builder::new()
        .prefix("study.v2.")
        .suffix(".db")
        .tempfile_in(parent)
        .map_err(|_| io_error())?;
    let report = {
        let mut copy = Connection::open(temporary.path())?;
        {
            let backup = Backup::new(&original, &mut copy)?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match backup.step(256)? {
                    StepResult::Done => break,
                    StepResult::More if Instant::now() < deadline => {}
                    StepResult::Busy | StepResult::Locked if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    _ => {
                        return Err(AppError::new(
                            "StorageBusy",
                            "전환 사본 준비 시간이 초과됐어요. 다시 시도해주세요.",
                            true,
                        ))
                    }
                }
            }
        }
        copy.execute_batch(
            "PRAGMA foreign_keys=OFF; PRAGMA synchronous=FULL; PRAGMA journal_mode=DELETE;",
        )?;
        let report = migrate_copy(&mut copy)?;
        copy.execute_batch("PRAGMA foreign_keys=ON;")?;
        integrity(&copy)?;
        copy.close().map_err(|_| io_error())?;
        // Reopen before publishing, so a failed output is never advertised as usable.
        let verified =
            Connection::open_with_flags(temporary.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        integrity(&verified)?;
        if metadata(&verified, "db_schema_version")?.as_deref() != Some("2") {
            return Err(AppError::inconsistent());
        }
        report
    };
    temporary.as_file().sync_all().map_err(|_| io_error())?;
    temporary
        .persist_noclobber(destination)
        .map_err(|_| io_error())?;
    Ok(report)
}

struct LegacyItem {
    dataset: String,
    id: String,
    position: i64,
    question: String,
    criteria: Criteria,
    active: i64,
}
struct Response {
    exam: String,
    position: i64,
    dataset: String,
    item: String,
    question: String,
    criteria: Criteria,
    text: String,
    judgement: String,
    reason: Option<String>,
}
struct Exam {
    id: String,
    status: String,
    count: usize,
    order: Option<i64>,
    completed: Option<String>,
    scope: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Progress {
    target: bool,
    streak: i64,
    last: String,
    exam: String,
}
type ProgressKey = (String, String);

fn legacy_criteria(answer: String, aliases: String) -> AppResult<Criteria> {
    let accepted_answers: Vec<String> =
        serde_json::from_str(&aliases).map_err(|_| AppError::inconsistent())?;
    // v1 did not impose new byte-size limits. Preserve all valid legacy answers.
    if !is_valid_answer(&answer) || accepted_answers.iter().any(|a| !is_valid_answer(a)) {
        return Err(AppError::inconsistent());
    }
    Ok(Criteria::ShortAnswer {
        answer,
        accepted_answers,
    })
}
fn valid_time(time: &str) -> AppResult<()> {
    chrono::DateTime::parse_from_rfc3339(time).map_err(|_| AppError::inconsistent())?;
    Ok(())
}

pub(crate) fn migrate_copy(db: &mut Connection) -> AppResult<MigrationReport> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    integrity(&tx)?;
    if metadata(&tx, "application_id")?.as_deref() != Some("StudyApp")
        || metadata(&tx, "initial_data_installed")?.as_deref() != Some("1")
    {
        return Err(AppError::inconsistent());
    }
    if metadata(&tx, "db_schema_version")?.as_deref() != Some("1") {
        return Err(AppError::new(
            "UnsupportedDbVersion",
            "버전 1 저장소의 사본만 전환할 수 있어요.",
            false,
        ));
    }
    let learner = metadata(&tx, "local_learner_id")?.ok_or_else(AppError::inconsistent)?;
    validate_exam_id(&learner).map_err(|_| AppError::inconsistent())?;
    let learners: Vec<String> = tx
        .prepare("SELECT learner_id FROM learners")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    if learners != [learner.clone()] {
        return Err(AppError::inconsistent());
    }
    {
        let mut statement = tx.prepare("SELECT dataset_id,title,metadata_json FROM datasets")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let extra: Option<String> = row.get(2)?;
            if !is_valid_id(&id) || is_unanswered(&title) {
                return Err(AppError::inconsistent());
            }
            if let Some(text) = extra {
                let value: serde_json::Value =
                    serde_json::from_str(&text).map_err(|_| AppError::inconsistent())?;
                if !value.is_object() {
                    return Err(AppError::inconsistent());
                }
            }
        }
        let payloads: Vec<String> = tx
            .prepare("SELECT payload_json FROM supplements")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for text in payloads {
            let value: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| AppError::inconsistent())?;
            if !value.is_object() {
                return Err(AppError::inconsistent());
            }
        }
    }
    let mut items = Vec::new();
    {
        let mut s=tx.prepare("SELECT dataset_id,item_id,position,question,answer,accepted_answers_json,is_active FROM items ORDER BY dataset_id,item_id")?;
        let mut rows = s.query([])?;
        while let Some(r) = rows.next()? {
            let item = LegacyItem {
                dataset: r.get(0)?,
                id: r.get(1)?,
                position: r.get(2)?,
                question: r.get(3)?,
                criteria: legacy_criteria(r.get(4)?, r.get(5)?)?,
                active: r.get(6)?,
            };
            if !is_valid_id(&item.dataset)
                || !is_valid_id(&item.id)
                || is_unanswered(&item.question)
            {
                return Err(AppError::inconsistent());
            }
            items.push(item);
        }
    }
    let mut exams = HashMap::new();
    {
        let mut s=tx.prepare("SELECT exam_id,learner_id,status,question_count,completion_order,completed_at,dataset_ids_json,dataset_titles_snapshot_json,count_mode,order_mode,review_mode,revision,current_position,started_at,saved_at,submitted_at,grading_rule_version FROM exams")?;
        let mut rows = s.query([])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let owner: String = r.get(1)?;
            let status: String = r.get(2)?;
            let count: u32 = r.get(3)?;
            let scope: Vec<String> = serde_json::from_str(&r.get::<_, String>(6)?)
                .map_err(|_| AppError::inconsistent())?;
            let titles: Vec<String> = serde_json::from_str(&r.get::<_, String>(7)?)
                .map_err(|_| AppError::inconsistent())?;
            let count_mode: String = r.get(8)?;
            StartExamRequest {
                exam_id: id.clone(),
                dataset_ids: scope.clone(),
                count: if count_mode == "count" {
                    Some(count)
                } else {
                    None
                },
                count_mode,
                order_mode: r.get(9)?,
                review_mode: r.get(10)?,
            }
            .validate()
            .map_err(|_| AppError::inconsistent())?;
            let revision: i64 = r.get(11)?;
            let position: u32 = r.get(12)?;
            for index in [13, 14] {
                valid_time(&r.get::<_, String>(index)?)?;
            }
            let submitted: Option<String> = r.get(15)?;
            if let Some(t) = &submitted {
                valid_time(t)?;
            }
            let completed: Option<String> = r.get(5)?;
            if let Some(t) = &completed {
                valid_time(t)?;
            }
            let order: Option<i64> = r.get(4)?;
            let rule: String = r.get(16)?;
            if owner != learner
                || count == 0
                || position >= count
                || !(0..=9_007_199_254_740_991).contains(&revision)
                || titles.len() != scope.len()
                || titles.iter().any(|t| is_unanswered(t))
                || rule != crate::exam::RULE
                || !matches!(
                    (
                        status.as_str(),
                        submitted.is_some(),
                        completed.is_some(),
                        order
                    ),
                    ("in_progress", false, false, None)
                        | ("submitted", true, false, None)
                        | ("completed", true, true, Some(1..))
                )
            {
                return Err(AppError::inconsistent());
            }
            exams.insert(
                id.clone(),
                Exam {
                    id,
                    status,
                    count: count as usize,
                    order,
                    completed,
                    scope,
                },
            );
        }
    }
    let mut responses = Vec::new();
    let mut response_indices: HashMap<String, Vec<usize>> = HashMap::new();
    {
        let mut s=tx.prepare("SELECT exam_id,position,dataset_id,item_id,question_snapshot,answer_snapshot,accepted_answers_snapshot_json,response_text,judgement,incorrect_reason FROM exam_responses ORDER BY exam_id,position")?;
        let mut rows = s.query([])?;
        while let Some(r) = rows.next()? {
            let response = Response {
                exam: r.get(0)?,
                position: r.get(1)?,
                dataset: r.get(2)?,
                item: r.get(3)?,
                question: r.get(4)?,
                criteria: legacy_criteria(r.get(5)?, r.get(6)?)?,
                text: r.get(7)?,
                judgement: r.get(8)?,
                reason: r.get(9)?,
            };
            let exam = exams
                .get(&response.exam)
                .ok_or_else(AppError::inconsistent)?;
            if !is_valid_id(&response.dataset)
                || !is_valid_id(&response.item)
                || is_unanswered(&response.question)
                || !exam.scope.contains(&response.dataset)
            {
                return Err(AppError::inconsistent());
            }
            if exam.status == "completed" {
                let Criteria::ShortAnswer {
                    answer,
                    accepted_answers,
                } = &response.criteria
                else {
                    unreachable!()
                };
                let expected = grade(&response.text, answer, accepted_answers);
                if response.judgement != expected.0 || response.reason.as_deref() != expected.1 {
                    return Err(AppError::inconsistent());
                }
            } else if response.judgement != "ungraded" || response.reason.is_some() {
                return Err(AppError::inconsistent());
            }
            response_indices
                .entry(response.exam.clone())
                .or_default()
                .push(responses.len());
            responses.push(response);
        }
    }
    for exam in exams.values() {
        let indices = response_indices
            .get(&exam.id)
            .ok_or_else(AppError::inconsistent)?;
        if indices.len() != exam.count
            || indices
                .iter()
                .enumerate()
                .any(|(i, p)| responses[*p].position != i as i64)
        {
            return Err(AppError::inconsistent());
        }
    }
    let mut completed: Vec<_> = exams.values().filter(|e| e.status == "completed").collect();
    completed.sort_by_key(|e| e.order);
    let mut expected: BTreeMap<ProgressKey, Progress> = BTreeMap::new();
    for exam in &completed {
        for &index in &response_indices[&exam.id] {
            let r = &responses[index];
            let p = expected
                .entry((r.dataset.clone(), r.item.clone()))
                .or_insert(Progress {
                    target: false,
                    streak: 0,
                    last: r.judgement.clone(),
                    exam: r.exam.clone(),
                });
            if r.judgement == "incorrect" {
                p.target = true;
                p.streak = 0;
            } else if p.target {
                p.streak += 1;
                if p.streak == 3 {
                    p.target = false;
                }
            }
            p.last = r.judgement.clone();
            p.exam = r.exam.clone();
        }
    }
    let mut existing = BTreeMap::new();
    {
        let mut s=tx.prepare("SELECT learner_id,dataset_id,item_id,is_review_target,correct_streak,last_result,last_completed_exam_id FROM item_progress")?;
        let mut rows = s.query([])?;
        while let Some(r) = rows.next()? {
            let owner: String = r.get(0)?;
            if owner != learner {
                return Err(AppError::inconsistent());
            }
            existing.insert(
                (r.get(1)?, r.get(2)?),
                Progress {
                    target: r.get(3)?,
                    streak: r.get(4)?,
                    last: r.get(5)?,
                    exam: r.get(6)?,
                },
            );
        }
    }
    if existing != expected {
        return Err(AppError::inconsistent());
    }
    tx.execute_batch(V2)?;
    for item in &items {
        let criteria =
            serde_json::to_string(&item.criteria).map_err(|_| AppError::inconsistent())?;
        tx.execute(
            "INSERT INTO items VALUES (?1,?2,?3,'short_answer',?4,?5,?6)",
            params![
                item.dataset,
                item.id,
                item.position,
                item.question,
                criteria,
                item.active
            ],
        )?;
    }
    let mut assessments = 0;
    for r in &responses {
        let criteria = serde_json::to_string(&r.criteria).map_err(|_| AppError::inconsistent())?;
        tx.execute(
            "INSERT INTO exam_responses VALUES (?1,?2,?3,?4,'short_answer',?5,?6,?7)",
            params![r.exam, r.position, r.dataset, r.item, r.question, criteria, r.text],
        )?;
        if let Some(time) = &exams[&r.exam].completed {
            let id = Uuid::new_v4().to_string();
            tx.execute("INSERT INTO assessment_events VALUES (?1,?2,?3,1,NULL,?4,?5,'migration',?6,NULL,NULL)",params![id,r.exam,r.position,r.judgement,r.reason,time])?;
            tx.execute(
                "INSERT INTO response_assessments VALUES (?1,?2,?3,?4,1,?5)",
                params![r.exam, r.position, r.judgement, r.reason, id],
            )?;
            assessments += 1;
        } else {
            tx.execute(
                "INSERT INTO response_assessments VALUES (?1,?2,'ungraded',NULL,0,NULL)",
                params![r.exam, r.position],
            )?;
        }
    }
    for exam in &completed {
        tx.execute(
            "INSERT INTO learning_events VALUES (?1,?2,?3,'exam_attempt',?4,NULL,NULL,?5)",
            params![
                learner,
                exam.order,
                Uuid::new_v4().to_string(),
                exam.id,
                exam.completed
            ],
        )?;
    }
    for ((dataset, item), p) in expected {
        tx.execute(
            "INSERT INTO item_progress VALUES (?1,?2,?3,?4,?5,?6,?7,1)",
            params![learner, dataset, item, p.target, p.streak, p.last, p.exam],
        )?;
    }
    integrity(&tx)?;
    tx.execute(
        "UPDATE app_metadata SET value='2' WHERE key='db_schema_version'",
        [],
    )?;
    tx.commit()?;
    Ok(MigrationReport {
        item_count: items.len(),
        response_count: responses.len(),
        assessment_count: assessments,
        attempt_count: completed.len(),
    })
}

#[cfg(test)]
pub(crate) mod tests;
