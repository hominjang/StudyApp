use super::*;
use crate::{
    learning_format::{load_catalog, ParsedPackage},
    storage::AppPaths,
};
use std::fs;

impl AssessmentStore {
    /// Fresh installations only. Existing files are opened, never replaced or upgraded here.
    pub fn initialize(paths: &AppPaths) -> AppResult<Self> {
        let target = paths.db_dir.join("study.db");
        if target.exists() {
            return Self::open(&target);
        }
        let packages = load_catalog(&paths.initial_dir)?;
        fs::create_dir_all(&paths.db_dir)
            .map_err(|_| AppError::new("StorageFailure", "저장 위치를 준비하지 못했어요.", true))?;
        let temporary = tempfile::Builder::new()
            .prefix("study.init.v2.")
            .suffix(".db")
            .tempfile_in(&paths.db_dir)
            .map_err(|_| AppError::inconsistent())?;
        {
            let mut db = Connection::open(temporary.path())?;
            db.execute_batch(include_str!("../../migrations/V001_Initial.sql"))?;
            let learner = Uuid::new_v4().to_string();
            db.execute(
                "INSERT INTO learners VALUES (?1,?2)",
                params![learner, Utc::now().to_rfc3339()],
            )?;
            db.execute("INSERT INTO app_metadata VALUES ('application_id','StudyApp'),('db_schema_version','1'),('initial_data_installed','1'),('local_learner_id',?1)",[&learner])?;
            crate::migration::migrate_copy(&mut db)?;
            db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
            seed(&mut db, &packages)?;
            if db.prepare("PRAGMA foreign_key_check")?.exists([])?
                || db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? != "ok"
            {
                return Err(AppError::inconsistent());
            }
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| AppError::inconsistent())?;
        if let Err(error) = temporary.persist_noclobber(&target) {
            if !target.is_file() {
                return Err(AppError::new(
                    "StorageFailure",
                    "저장소 초기화를 마치지 못했어요.",
                    true,
                ));
            }
            drop(error);
        }
        Self::open(&target)
    }
}

fn seed(db: &mut Connection, packages: &[ParsedPackage]) -> AppResult<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (position, p) in packages.iter().enumerate() {
        let metadata = p
            .supplement
            .as_ref()
            .and_then(|s| s.dataset_metadata.as_ref())
            .map(json)
            .transpose()?;
        let encoded = crate::learning_format::export_learning_v2(&p.learning)?;
        let learning: serde_json::Value =
            serde_json::from_slice(&encoded).map_err(|_| corrupt())?;
        let package = serde_json::json!({"learning":learning,"supplement":p.supplement,"learning_links":p.links});
        let hash = format!("{:x}", Sha256::digest(json(&package)?.as_bytes()));
        tx.execute(
            "INSERT INTO datasets VALUES (?1,?2,?3,1,?4,?5)",
            params![
                p.learning.dataset_id,
                p.learning.title,
                position as i64,
                hash,
                metadata
            ],
        )?;
        for (index, item) in p.learning.items.iter().enumerate() {
            let criteria = item.criteria();
            let kind = match criteria {
                Criteria::ShortAnswer { .. } => "short_answer",
                Criteria::Descriptive { .. } => "descriptive",
            };
            tx.execute(
                "INSERT INTO items VALUES (?1,?2,?3,?4,?5,?6,1)",
                params![
                    p.learning.dataset_id,
                    item.id(),
                    index as i64,
                    kind,
                    item.question(),
                    json(&criteria)?
                ],
            )?;
        }
        if let Some(s) = &p.supplement {
            for value in &s.items {
                let mut payload = value.as_object().cloned().ok_or_else(corrupt)?;
                let id = payload
                    .remove("id")
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .ok_or_else(corrupt)?;
                tx.execute(
                    "INSERT INTO supplements VALUES (?1,?2,?3)",
                    params![p.learning.dataset_id, id, json(&payload)?],
                )?;
            }
        }
        for (index, link) in p.links.iter().enumerate() {
            tx.execute(
                "INSERT INTO item_learning_links VALUES (?1,?2,?3,?4)",
                params![
                    p.learning.dataset_id,
                    link.item_id,
                    index as i64,
                    json(link)?
                ],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}
