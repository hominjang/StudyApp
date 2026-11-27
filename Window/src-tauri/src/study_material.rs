//! Explicit reading mode: current active learning content, with no attempt/progress writes.
use crate::{
    error::{AppError, AppResult},
    learning_format::{validate_criteria, Criteria},
    validation::is_valid_id,
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudyRequest {
    pub dataset_id: Option<String>,
    pub search: String,
    pub offset: u32,
    pub limit: u32,
}
#[derive(Serialize)]
pub struct StudyItem {
    pub dataset_revision: u64,
    pub dataset_id: String,
    pub item_id: String,
    pub dataset_title: String,
    pub question: String,
    pub criteria: Criteria,
    pub explanation: Option<String>,
}
#[derive(Serialize)]
pub struct StudyPage {
    pub total: u32,
    pub offset: u32,
    pub limit: u32,
    pub items: Vec<StudyItem>,
}

pub fn browse(db: &mut Connection, version: u8, request: StudyRequest) -> AppResult<StudyPage> {
    if !matches!(version, 1 | 2)
        || !(1..=50).contains(&request.limit)
        || request.offset % request.limit != 0
        || request.search.len() > 512
        || request
            .dataset_id
            .as_ref()
            .is_some_and(|id| !is_valid_id(id))
    {
        return Err(AppError::validation("study_scope"));
    }
    let tx = db.transaction()?;
    if let Some(id) = &request.dataset_id {
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM datasets WHERE dataset_id=?1)",
            [id],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(AppError::new(
                "NotFound",
                "학습 자료가 없어요. 자료 목록을 다시 확인해주세요.",
                false,
            ));
        }
    }
    const FILTER:&str=" WHERE i.is_active=1 AND (?1 IS NULL OR i.dataset_id=?1) AND instr(lower(i.question),lower(?2))>0";
    let count: i64 = tx.query_row(
        &format!("SELECT COUNT(*) FROM items i{FILTER}"),
        params![request.dataset_id, request.search],
        |r| r.get(0),
    )?;
    let total = u32::try_from(count).map_err(|_| AppError::inconsistent())?;
    let offset = request
        .offset
        .min(total.saturating_sub(1) / request.limit * request.limit);
    let fields = if version == 2 {
        "i.criteria_json,i.type"
    } else {
        "i.answer,i.accepted_answers_json"
    };
    let sql=format!("SELECT i.dataset_id,i.item_id,d.title,i.question,{fields},s.payload_json,d.revision FROM items i JOIN datasets d ON d.dataset_id=i.dataset_id LEFT JOIN supplements s ON s.dataset_id=i.dataset_id AND s.item_id=i.item_id{FILTER} ORDER BY d.position,d.dataset_id,i.position,i.item_id LIMIT ?3 OFFSET ?4");
    let mut items = Vec::new();
    {
        let mut statement = tx.prepare(&sql)?;
        let mut rows = statement.query(params![
            request.dataset_id,
            request.search,
            request.limit,
            offset
        ])?;
        while let Some(row) = rows.next()? {
            let criteria: Criteria = if version == 2 {
                let parsed: Criteria = serde_json::from_str(&row.get::<_, String>(4)?)
                    .map_err(|_| AppError::inconsistent())?;
                let kind = row.get::<_, String>(5)?;
                if !matches!(
                    (&parsed, kind.as_str()),
                    (Criteria::ShortAnswer { .. }, "short_answer")
                        | (Criteria::Descriptive { .. }, "descriptive")
                ) {
                    return Err(AppError::inconsistent());
                }
                parsed
            } else {
                Criteria::ShortAnswer {
                    answer: row.get(4)?,
                    accepted_answers: serde_json::from_str(&row.get::<_, String>(5)?)
                        .map_err(|_| AppError::inconsistent())?,
                }
            };
            validate_criteria(&criteria).map_err(|_| AppError::inconsistent())?;
            let extra: Option<String> = row.get(6)?;
            let explanation = extra
                .map(|v| {
                    serde_json::from_str::<serde_json::Value>(&v)
                        .map_err(|_| AppError::inconsistent())
                })
                .transpose()?
                .and_then(|v| {
                    v.get("explanation")
                        .and_then(|x| x.as_str())
                        .map(str::to_owned)
                });
            let revision: i64 = row.get(7)?;
            if !(1..=9_007_199_254_740_991).contains(&revision) {
                return Err(AppError::inconsistent());
            }
            items.push(StudyItem {
                dataset_revision: revision as u64,
                dataset_id: row.get(0)?,
                item_id: row.get(1)?,
                dataset_title: row.get(2)?,
                question: row.get(3)?,
                criteria,
                explanation,
            });
        }
    }
    tx.commit()?;
    Ok(StudyPage {
        total,
        offset,
        limit: request.limit,
        items,
    })
}

#[cfg(test)]
mod tests;
