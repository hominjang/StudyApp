use super::*;

#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct Progress {
    pub target: bool,
    pub streak: u8,
    pub last: Option<String>,
    pub exam: Option<String>,
    pub revision: u64,
}

pub(super) fn compute_progress(
    db: &Connection,
    learner: &str,
    dataset: &str,
    item: &str,
) -> AppResult<Progress> {
    compute(db, learner, dataset, item).map_err(|_| corrupt())
}
fn compute(db: &Connection, learner: &str, dataset: &str, item: &str) -> AppResult<Progress> {
    let mut state = Progress::default();
    let mut applied = 0;
    let mut previous_order = 0;
    let mut statement=db.prepare("SELECT event_seq,kind,exam_id,dataset_id,item_id,event_id,recorded_at FROM learning_events WHERE learner_id=?1 ORDER BY event_seq")?;
    let mut rows = statement.query([learner])?;
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let kind: String = row.get(1)?;
        let exam: Option<String> = row.get(2)?;
        let d: Option<String> = row.get(3)?;
        let i: Option<String> = row.get(4)?;
        let event: String = row.get(5)?;
        let recorded: String = row.get(6)?;
        if uuid(&event).is_err() || chrono::DateTime::parse_from_rfc3339(&recorded).is_err() {
            return Err(corrupt());
        }
        if seq < 1 || seq as u64 > MAX_REV {
            return Err(corrupt());
        }
        match (kind.as_str(), exam, d, i) {
            ("manual_review_add", None, Some(d), Some(i)) => {
                if !is_valid_id(&d) || !is_valid_id(&i) {
                    return Err(corrupt());
                }
                if d == dataset && i == item {
                    state.target = true;
                    state.streak = 0;
                }
            }
            ("exam_attempt", Some(exam), None, None) => {
                uuid(&exam)?;
                completed_exam(db, learner, &exam)?;
                let order: i64 = db.query_row(
                    "SELECT completion_order FROM exams WHERE exam_id=?1",
                    [&exam],
                    |r| r.get(0),
                )?;
                if order <= previous_order {
                    return Err(corrupt());
                }
                previous_order = order;
                let position:Option<u32>=db.query_row("SELECT position FROM exam_responses WHERE exam_id=?1 AND dataset_id=?2 AND item_id=?3",params![exam,dataset,item],|r|r.get(0)).optional()?;
                if let Some(position) = position {
                    applied += 1;
                    let response = load_response(db, &exam, position)?;
                    match response.judgement.as_str() {
                        "ungraded" => continue,
                        "incorrect" | "partial" => {
                            state.target = true;
                            state.streak = 0;
                        }
                        "correct" => {
                            if state.target {
                                state.streak += 1;
                                if state.streak == 3 {
                                    state.target = false;
                                }
                            }
                        }
                        _ => return Err(corrupt()),
                    }
                    state.last = Some(response.judgement);
                    state.exam = Some(exam);
                }
            }
            _ => return Err(corrupt()),
        }
    }
    let expected:i64=db.query_row("SELECT COUNT(*) FROM exam_responses r JOIN exams e ON e.exam_id=r.exam_id WHERE e.learner_id=?1 AND e.status='completed' AND r.dataset_id=?2 AND r.item_id=?3",params![learner,dataset,item],|r|r.get(0))?;
    if applied != expected {
        return Err(corrupt());
    }
    Ok(state)
}

pub(super) fn checked_progress(
    db: &Connection,
    learner: &str,
    dataset: &str,
    item: &str,
) -> AppResult<Progress> {
    let mut expected = compute_progress(db, learner, dataset, item)?;
    let cached:Option<Progress>=db.query_row("SELECT is_review_target,correct_streak,last_result,last_completed_exam_id,progress_revision FROM item_progress WHERE learner_id=?1 AND dataset_id=?2 AND item_id=?3",params![learner,dataset,item],|r|Ok(Progress{target:r.get(0)?,streak:r.get(1)?,last:r.get(2)?,exam:r.get(3)?,revision:sql_revision(r,4)?})).optional().map_err(|_|corrupt())?;
    if let Some(cached) = cached {
        if cached.revision == 0 || cached.revision > MAX_REV {
            return Err(corrupt());
        }
        expected.revision = cached.revision;
        if expected != cached {
            return Err(corrupt());
        }
    } else if expected != Progress::default() {
        return Err(corrupt());
    }
    Ok(expected)
}

pub(super) fn save_progress(
    db: &Connection,
    learner: &str,
    dataset: &str,
    item: &str,
    p: Progress,
    previous: u64,
) -> AppResult<u64> {
    let revision = previous
        .checked_add(1)
        .filter(|n| *n <= MAX_REV)
        .ok_or_else(corrupt)?;
    db.execute("INSERT INTO item_progress VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(learner_id,dataset_id,item_id) DO UPDATE SET is_review_target=excluded.is_review_target,correct_streak=excluded.correct_streak,last_result=excluded.last_result,last_completed_exam_id=excluded.last_completed_exam_id,progress_revision=excluded.progress_revision",params![learner,dataset,item,p.target,p.streak,p.last,p.exam,revision as i64])?;
    Ok(revision)
}
