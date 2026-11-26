use crate::{
    error::{AppError, AppResult},
    package::{load_initial, validate_package, Package},
};
use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::Duration};
use uuid::Uuid;

const SCHEMA: &str = include_str!("../migrations/V001_Initial.sql");

#[derive(Clone)]
pub struct AppPaths {
    pub db_dir: PathBuf,
    pub initial_dir: PathBuf,
}

#[derive(Clone, Serialize)]
pub struct DatasetSummary {
    pub dataset_id: String,
    pub title: String,
    pub position: usize,
    pub question_count: usize,
    pub review_target_count: usize,
}

#[derive(Clone, Serialize)]
pub struct AppStatus {
    pub runtime: &'static str,
    pub storage: &'static str,
    pub dataset_count: usize,
    pub question_count: usize,
}

pub struct Store {
    connection: Connection,
    learner_id: String,
}

fn configure(connection: &Connection) -> AppResult<()> {
    connection.busy_timeout(Duration::from_secs(3))?;
    connection.execute_batch(
        "PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL; PRAGMA journal_mode=DELETE;",
    )?;
    if connection.pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))? != 1
        || connection.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))? != 2
        || connection.pragma_query_value(None, "busy_timeout", |row| row.get::<_, i64>(0))? != 3000
        || connection.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?
            != "delete"
    {
        return Err(AppError::inconsistent());
    }
    Ok(())
}

fn metadata(connection: &Connection, key: &str) -> AppResult<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM app_metadata WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| AppError::inconsistent())
}

fn canonicalize(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut values) => {
            let mut keys: Vec<String> = values.keys().cloned().collect();
            keys.sort();
            serde_json::Value::Object(
                keys.into_iter()
                    .map(|key| {
                        let value = canonicalize(values.remove(&key).unwrap());
                        (key, value)
                    })
                    .collect(),
            )
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonicalize).collect())
        }
        other => other,
    }
}

fn ensure_empty(connection: &Connection) -> AppResult<()> {
    for table in [
        "learners",
        "datasets",
        "items",
        "supplements",
        "exams",
        "exam_responses",
        "item_progress",
    ] {
        // table은 위의 고정 문자열만 사용한다.
        let count: i64 =
            connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
        if count != 0 {
            return Err(AppError::inconsistent());
        }
    }
    if metadata(connection, "local_learner_id")?.is_some() {
        return Err(AppError::inconsistent());
    }
    Ok(())
}

fn seed(connection: &mut Connection, packages: &[Package], create_schema: bool) -> AppResult<()> {
    for package in packages {
        validate_package(package)?;
    }
    if packages.is_empty() {
        return Err(AppError::validation("datasets"));
    }
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if create_schema {
        tx.execute_batch(SCHEMA)?;
        tx.execute("INSERT INTO app_metadata VALUES ('application_id','StudyApp'),('db_schema_version','1')", [])?;
    }
    ensure_empty(&tx)?;
    let learner_id = loop {
        let id = Uuid::new_v4().to_string();
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM learners WHERE learner_id=?1)",
            [&id],
            |row| row.get::<_, bool>(0),
        )? {
            break id;
        }
    };
    let now = Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO learners(learner_id,created_at) VALUES (?1,?2)",
        params![learner_id, now],
    )?;
    for (position, package) in packages.iter().enumerate() {
        let data = &package.learning;
        let value = serde_json::to_value(package).map_err(|_| AppError::validation("package"))?;
        let serialized = serde_json::to_vec(&canonicalize(value))
            .map_err(|_| AppError::validation("package"))?;
        let hash = format!("{:x}", Sha256::digest(&serialized));
        let dataset_metadata = package
            .supplement
            .as_ref()
            .and_then(|s| s.dataset_metadata.as_ref())
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| AppError::validation("dataset_metadata"))?;
        tx.execute("INSERT INTO datasets(dataset_id,title,position,revision,content_hash,metadata_json) VALUES (?1,?2,?3,1,?4,?5)",
            params![data.dataset_id, data.title, position as i64, hash, dataset_metadata])?;
        for (index, item) in data.items.iter().enumerate() {
            let aliases = serde_json::to_string(&item.accepted_answers)
                .map_err(|_| AppError::validation("accepted_answers"))?;
            tx.execute("INSERT INTO items(dataset_id,item_id,position,question,answer,accepted_answers_json,is_active) VALUES (?1,?2,?3,?4,?5,?6,1)",
                params![data.dataset_id,item.id,index as i64,item.question,item.answer,aliases])?;
        }
        if let Some(supplement) = &package.supplement {
            for value in &supplement.items {
                let mut object = value
                    .as_object()
                    .cloned()
                    .ok_or_else(|| AppError::validation("supplement.items"))?;
                let id = object
                    .remove("id")
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .ok_or_else(|| AppError::validation("supplement.items.id"))?;
                let payload = serde_json::to_string(&object)
                    .map_err(|_| AppError::validation("supplement.items"))?;
                tx.execute(
                    "INSERT INTO supplements VALUES (?1,?2,?3)",
                    params![data.dataset_id, id, payload],
                )?;
            }
        }
    }
    tx.execute(
        "INSERT INTO app_metadata(key,value) VALUES ('local_learner_id',?1)",
        [&learner_id],
    )?;
    tx.execute("INSERT INTO app_metadata(key,value) VALUES ('initial_data_installed','1') ON CONFLICT(key) DO UPDATE SET value='1'", [])?;
    tx.commit()?;
    Ok(())
}

impl Store {
    pub fn open(paths: &AppPaths) -> AppResult<Self> {
        let final_path = paths.db_dir.join("study.db");
        if !final_path.exists() {
            let packages = load_initial(&paths.initial_dir)?;
            fs::create_dir_all(&paths.db_dir).map_err(|_| {
                AppError::new(
                    "StorageFailure",
                    "학습 자료 저장 위치를 준비하지 못했어요.",
                    true,
                )
            })?;
            let temporary = tempfile::Builder::new()
                .prefix("study.init.")
                .suffix(".db")
                .tempfile_in(&paths.db_dir)
                .map_err(|_| {
                    AppError::new("StorageFailure", "임시 저장소를 준비하지 못했어요.", true)
                })?;
            {
                let mut connection = Connection::open(temporary.path())?;
                configure(&connection)?;
                seed(&mut connection, &packages, true)?;
                if connection
                    .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?
                    != "ok"
                {
                    return Err(AppError::inconsistent());
                }
            }
            if let Err(error) = temporary.persist_noclobber(&final_path) {
                if !final_path.is_file() {
                    return Err(AppError::new(
                        "StorageFailure",
                        "저장소 초기화를 완료하지 못했어요.",
                        true,
                    ));
                }
                // 다른 실행이 정상 DB를 먼저 만들었다면 그 DB를 아래에서 검증한다.
                drop(error);
            }
        }
        let mut connection =
            Connection::open_with_flags(&final_path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        configure(&connection)?;
        if metadata(&connection, "application_id")?.as_deref() != Some("StudyApp") {
            return Err(AppError::inconsistent());
        }
        if metadata(&connection, "db_schema_version")?.as_deref() != Some("1") {
            return Err(AppError::new(
                "UnsupportedDbVersion",
                "현재 앱이 지원하지 않는 저장소 버전이에요.",
                false,
            ));
        }
        if connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?
            != "ok"
        {
            return Err(AppError::inconsistent());
        }
        if metadata(&connection, "initial_data_installed")?.as_deref() != Some("1") {
            // 자료 파일 유무와 관계없이 기존 행이 있으면 초기화를 거부한다.
            ensure_empty(&connection)?;
            let packages = load_initial(&paths.initial_dir)?;
            seed(&mut connection, &packages, false)?;
        }
        if connection.prepare("PRAGMA foreign_key_check")?.exists([])? {
            return Err(AppError::inconsistent());
        }
        let learner_id =
            metadata(&connection, "local_learner_id")?.ok_or_else(AppError::inconsistent)?;
        let uuid = Uuid::parse_str(&learner_id).map_err(|_| AppError::inconsistent())?;
        if uuid.get_version_num() != 4 || uuid.to_string() != learner_id {
            return Err(AppError::inconsistent());
        }
        let count: i64 =
            connection.query_row("SELECT COUNT(*) FROM learners", [], |row| row.get(0))?;
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM learners WHERE learner_id=?1)",
            [&learner_id],
            |row| row.get(0),
        )?;
        if count != 1 || !exists {
            return Err(AppError::inconsistent());
        }
        Ok(Self {
            connection,
            learner_id,
        })
    }

    pub fn list_datasets(&mut self) -> AppResult<Vec<DatasetSummary>> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let rows = {
            let mut statement = tx.prepare("SELECT d.dataset_id,d.title,d.position,COUNT(i.item_id),
                (SELECT COUNT(*) FROM item_progress p JOIN items pi ON pi.dataset_id=p.dataset_id AND pi.item_id=p.item_id
                 WHERE p.learner_id=?1 AND p.dataset_id=d.dataset_id AND p.is_review_target=1 AND pi.is_active=1)
                FROM datasets d LEFT JOIN items i ON i.dataset_id=d.dataset_id AND i.is_active=1
                GROUP BY d.dataset_id ORDER BY d.position,d.dataset_id")?;
            let rows = statement
                .query_map([&self.learner_id], |row| {
                    Ok(DatasetSummary {
                        dataset_id: row.get(0)?,
                        title: row.get(1)?,
                        position: row.get::<_, u32>(2)? as usize,
                        question_count: row.get::<_, u32>(3)? as usize,
                        review_target_count: row.get::<_, u32>(4)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
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
}

mod exams;
mod history;
#[cfg(test)]
mod tests;
