//! User-selected UTF-8 JSON only. No arbitrary filesystem access or record import.
use crate::{
    error::{AppError, AppResult},
    learning_format::{self, Criteria, LearningLink, ParsedPackage},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;
mod edit;
pub use edit::{EditMaterialRequest, EditMaterialResult, MaterialEditState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    pub learning_json: String,
    pub supplement_json: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct ImportPreview {
    pub token: String,
    pub dataset_id: String,
    pub title: String,
    pub schema_version: String,
    pub db_version: u8,
    pub existing: bool,
    pub changed: bool,
    pub question_count: usize,
    pub added: usize,
    pub modified: usize,
    pub deactivated: usize,
}
#[derive(Clone, Serialize)]
pub struct ImportResult {
    pub dataset_id: String,
    pub changed: bool,
    pub question_count: usize,
}
#[derive(Serialize)]
struct StoredItem {
    id: String,
    question: String,
    criteria: Criteria,
}
#[derive(Serialize)]
struct Snapshot {
    title: String,
    items: Vec<StoredItem>,
    metadata: Option<Value>,
    supplements: BTreeMap<String, Value>,
    links: Vec<LearningLink>,
}
struct Pending {
    preview: ImportPreview,
    revision: Option<i64>,
    desired: Snapshot,
}
#[derive(Default)]
pub struct ImportState {
    pending: Option<Pending>,
    applied: Option<(String, ImportResult)>,
}

fn conflict() -> AppError {
    AppError::new(
        "ImportConflict",
        "자료가 바뀌었어요. 파일을 다시 확인한 뒤 미리보기를 만들어주세요.",
        false,
    )
}
fn encode(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_string(value).map_err(|_| AppError::inconsistent())
}
fn decode<T: serde::de::DeserializeOwned>(text: &str) -> AppResult<T> {
    serde_json::from_str(text).map_err(|_| AppError::inconsistent())
}
fn revision(db: &Connection, id: &str) -> AppResult<Option<i64>> {
    Ok(db
        .query_row(
            "SELECT revision FROM datasets WHERE dataset_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?)
}
fn read_snapshot(db: &Connection, id: &str, version: u8) -> AppResult<Option<Snapshot>> {
    let info: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT title,metadata_json FROM datasets WHERE dataset_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((title, metadata)) = info else {
        return Ok(None);
    };
    let sql = if version == 2 {
        "SELECT item_id,question,criteria_json FROM items WHERE dataset_id=?1 AND is_active=1 ORDER BY position"
    } else {
        "SELECT item_id,question,answer,accepted_answers_json FROM items WHERE dataset_id=?1 AND is_active=1 ORDER BY position"
    };
    let mut stmt = db.prepare(sql)?;
    let mut rows = stmt.query([id])?;
    let mut items = Vec::new();
    while let Some(r) = rows.next()? {
        let criteria = if version == 2 {
            decode(&r.get::<_, String>(2)?)?
        } else {
            Criteria::ShortAnswer {
                answer: r.get(2)?,
                accepted_answers: decode(&r.get::<_, String>(3)?)?,
            }
        };
        items.push(StoredItem {
            id: r.get(0)?,
            question: r.get(1)?,
            criteria,
        });
    }
    let mut supplements = BTreeMap::new();
    for row in db
        .prepare(
            "SELECT item_id,payload_json FROM supplements WHERE dataset_id=?1 ORDER BY item_id",
        )?
        .query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
    {
        let (key, payload) = row?;
        supplements.insert(key, decode(&payload)?);
    }
    let mut links = Vec::new();
    if version == 2 {
        for row in db.prepare("SELECT payload_json FROM item_learning_links WHERE dataset_id=?1 ORDER BY position")?.query_map([id],|r|r.get::<_,String>(0))? {links.push(decode(&row?)?);}
    }
    Ok(Some(Snapshot {
        title,
        items,
        metadata: metadata.map(|v| decode(&v)).transpose()?,
        supplements,
        links,
    }))
}
fn desired(package: ParsedPackage, old: Option<&Snapshot>) -> AppResult<Snapshot> {
    let mut metadata = old.and_then(|s| s.metadata.clone());
    let mut supplements = old.map(|s| s.supplements.clone()).unwrap_or_default();
    let mut links = old.map(|s| s.links.clone()).unwrap_or_default();
    if let Some(s) = package.supplement {
        if s.dataset_metadata.is_some() {
            metadata = s.dataset_metadata;
        }
        for item in s.items {
            let mut object = item
                .as_object()
                .cloned()
                .ok_or_else(AppError::inconsistent)?;
            let id = object
                .remove("id")
                .and_then(|v| v.as_str().map(str::to_owned))
                .ok_or_else(AppError::inconsistent)?;
            supplements.insert(id, Value::Object(object));
        }
        if s.schema_version == "2.0" {
            links = package.links;
        }
    }
    Ok(Snapshot {
        title: package.learning.title,
        items: package
            .learning
            .items
            .iter()
            .map(|i| StoredItem {
                id: i.id().into(),
                question: i.question().into(),
                criteria: i.criteria(),
            })
            .collect(),
        metadata,
        supplements,
        links,
    })
}
impl ImportState {
    pub fn preview(
        &mut self,
        db: &mut Connection,
        version: u8,
        request: ImportRequest,
    ) -> AppResult<ImportPreview> {
        // Invalid replacement must not leave an older preview applicable.
        self.pending = None;
        let package = learning_format::parse_package(
            request.learning_json.as_bytes(),
            request.supplement_json.as_deref().map(str::as_bytes),
        )?;
        if version == 1 && package.learning.schema_version != "1.0" {
            return Err(AppError::new("UnsupportedFormat","현재 저장소에는 1.0 단답형 자료만 추가할 수 있어요. 서술형 2.0 자료는 저장소 전환 기능이 준비된 뒤 사용할 수 있어요.",false));
        }
        let id = package.learning.dataset_id.clone();
        let schema_version = package.learning.schema_version.clone();
        let tx = db.transaction()?;
        let rev = revision(&tx, &id)?;
        let old = read_snapshot(&tx, &id, version)?;
        let data = desired(package, old.as_ref())?;
        let old_items: BTreeMap<_, _> = old
            .as_ref()
            .map(|s| s.items.iter().map(|i| (i.id.as_str(), i)).collect())
            .unwrap_or_default();
        let ids: HashSet<_> = data.items.iter().map(|i| i.id.as_str()).collect();
        let added = data
            .items
            .iter()
            .filter(|i| !old_items.contains_key(i.id.as_str()))
            .count();
        let modified = data
            .items
            .iter()
            .filter(|i| {
                old_items
                    .get(i.id.as_str())
                    .is_some_and(|o| encode(o).ok() != encode(i).ok())
            })
            .count();
        let deactivated = old_items.keys().filter(|id| !ids.contains(**id)).count();
        let changed =
            old.as_ref().map(encode).transpose()?.as_deref() != Some(encode(&data)?.as_str());
        let preview = ImportPreview {
            token: Uuid::new_v4().to_string(),
            dataset_id: id,
            title: data.title.clone(),
            schema_version,
            db_version: version,
            existing: rev.is_some(),
            changed,
            question_count: data.items.len(),
            added,
            modified,
            deactivated,
        };
        tx.commit()?;
        self.pending = Some(Pending {
            preview: preview.clone(),
            revision: rev,
            desired: data,
        });
        Ok(preview)
    }
    pub fn apply(
        &mut self,
        db: &mut Connection,
        version: u8,
        token: &str,
    ) -> AppResult<ImportResult> {
        if let Some((saved, result)) = &self.applied {
            if saved == token {
                return Ok(result.clone());
            }
        }
        let pending = self
            .pending
            .as_ref()
            .filter(|p| p.preview.token == token && p.preview.db_version == version)
            .ok_or_else(conflict)?;
        let id = &pending.preview.dataset_id;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if revision(&tx, id)? != pending.revision {
            return Err(conflict());
        }
        let result = ImportResult {
            dataset_id: id.clone(),
            changed: pending.preview.changed,
            question_count: pending.desired.items.len(),
        };
        if result.changed {
            write_snapshot(&tx, version, id, pending.revision, &pending.desired)?;
        }
        tx.commit()?;
        self.applied = Some((token.into(), result.clone()));
        self.pending = None;
        Ok(result)
    }
}
fn write_snapshot(
    db: &Connection,
    version: u8,
    id: &str,
    rev: Option<i64>,
    data: &Snapshot,
) -> AppResult<()> {
    let hash = format!("{:x}", Sha256::digest(encode(data)?.as_bytes()));
    let metadata = data.metadata.as_ref().map(encode).transpose()?;
    if let Some(old) = rev {
        let next = old
            .checked_add(1)
            .filter(|n| *n <= 9_007_199_254_740_991)
            .ok_or_else(AppError::inconsistent)?;
        db.execute("UPDATE datasets SET title=?2,revision=?3,content_hash=?4,metadata_json=?5 WHERE dataset_id=?1",params![id,data.title,next,hash,metadata])?;
        db.execute("UPDATE items SET is_active=0 WHERE dataset_id=?1", [id])?;
    } else {
        db.execute("INSERT INTO datasets VALUES (?1,?2,(SELECT COALESCE(MAX(position),-1)+1 FROM datasets),1,?3,?4)",params![id,data.title,hash,metadata])?;
    }
    for (index, item) in data.items.iter().enumerate() {
        if version == 2 {
            let kind = match item.criteria {
                Criteria::ShortAnswer { .. } => "short_answer",
                Criteria::Descriptive { .. } => "descriptive",
            };
            db.execute("INSERT INTO items VALUES (?1,?2,?3,?4,?5,?6,1) ON CONFLICT(dataset_id,item_id) DO UPDATE SET position=excluded.position,type=excluded.type,question=excluded.question,criteria_json=excluded.criteria_json,is_active=1",params![id,item.id,index as i64,kind,item.question,encode(&item.criteria)?])?;
        } else if let Criteria::ShortAnswer {
            answer,
            accepted_answers,
        } = &item.criteria
        {
            db.execute("INSERT INTO items VALUES (?1,?2,?3,?4,?5,?6,1) ON CONFLICT(dataset_id,item_id) DO UPDATE SET position=excluded.position,question=excluded.question,answer=excluded.answer,accepted_answers_json=excluded.accepted_answers_json,is_active=1",params![id,item.id,index as i64,item.question,answer,encode(accepted_answers)?])?;
        } else {
            return Err(AppError::validation("items.type"));
        }
    }
    for (key, payload) in &data.supplements {
        db.execute("INSERT INTO supplements VALUES (?1,?2,?3) ON CONFLICT(dataset_id,item_id) DO UPDATE SET payload_json=excluded.payload_json",params![id,key,encode(payload)?])?;
    }
    if version == 2 {
        db.execute("DELETE FROM item_learning_links WHERE dataset_id=?1", [id])?;
        for (index, link) in data.links.iter().enumerate() {
            db.execute(
                "INSERT INTO item_learning_links VALUES (?1,?2,?3,?4)",
                params![id, link.item_id, index as i64, encode(link)?],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
