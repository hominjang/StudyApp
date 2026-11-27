use crate::{
    error::{AppError, AppResult},
    validation::{is_unanswered, is_valid_answer, is_valid_id},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path},
};

#[derive(Deserialize)]
struct Catalog {
    schema_version: String,
    datasets: Vec<CatalogEntry>,
}

#[derive(Deserialize)]
struct CatalogEntry {
    dataset_id: String,
    title: String,
    learning_file: String,
    supplement_file: Option<String>,
    question_count: usize,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Learning {
    pub schema_version: String,
    pub dataset_id: String,
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Item {
    pub id: String,
    pub question: String,
    pub answer: String,
    #[serde(default)]
    pub accepted_answers: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Supplement {
    pub schema_version: String,
    pub dataset_id: String,
    pub dataset_metadata: Option<Value>,
    pub items: Vec<Value>,
}

#[derive(Clone, Serialize)]
pub struct Package {
    pub learning: Learning,
    pub supplement: Option<Supplement>,
}

fn read_json<T: serde::de::DeserializeOwned>(
    root: &Path,
    relative: &str,
    field: &str,
) -> AppResult<T> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(AppError::validation(field));
    }
    let root = root.canonicalize().map_err(|_| {
        AppError::new(
            "DataUnavailable",
            "초기 학습 자료를 찾지 못했어요. 로컬 자료를 준비한 뒤 다시 시도해주세요.",
            true,
        )
    })?;
    let resolved = root.join(path).canonicalize().map_err(|_| {
        AppError::new(
            "DataUnavailable",
            "학습 자료 파일을 읽지 못했어요. 필요한 파일이 있는지 확인해주세요.",
            true,
        )
    })?;
    if !resolved.starts_with(&root) || !resolved.is_file() {
        return Err(AppError::validation(field));
    }
    let bytes = fs::read(resolved)
        .map_err(|_| AppError::new("DataUnavailable", "학습 자료 파일을 읽지 못했어요.", true))?;
    serde_json::from_slice(&bytes).map_err(|_| AppError::validation(field))
}

pub fn validate_package(package: &Package) -> AppResult<()> {
    let data = &package.learning;
    if data.schema_version != "1.0" {
        return Err(AppError::new(
            "UnsupportedFormat",
            "지원하는 학습 자료 형식은 1.0이에요.",
            false,
        ));
    }
    if !is_valid_id(&data.dataset_id) {
        return Err(AppError::validation("dataset_id"));
    }
    if is_unanswered(&data.title) || data.items.is_empty() {
        return Err(AppError::validation("title/items"));
    }
    let mut ids = HashSet::new();
    for (index, item) in data.items.iter().enumerate() {
        if !is_valid_id(&item.id) || !ids.insert(item.id.as_str()) {
            return Err(AppError::validation(format!("items[{index}].id")));
        }
        if is_unanswered(&item.question) {
            return Err(AppError::validation(format!("items[{index}].question")));
        }
        if !is_valid_answer(&item.answer) {
            return Err(AppError::validation(format!("items[{index}].answer")));
        }
        if item
            .accepted_answers
            .iter()
            .any(|value| !is_valid_answer(value))
        {
            return Err(AppError::validation(format!(
                "items[{index}].accepted_answers"
            )));
        }
    }
    if let Some(supplement) = &package.supplement {
        validate_supplement(supplement, "1.0", &data.dataset_id, &ids)?;
    }
    Ok(())
}

pub(crate) fn validate_supplement(
    supplement: &Supplement,
    version: &str,
    dataset_id: &str,
    ids: &HashSet<&str>,
) -> AppResult<()> {
    if supplement.schema_version != version || supplement.dataset_id != dataset_id {
        return Err(AppError::validation("supplement.schema_version/dataset_id"));
    }
    if supplement
        .dataset_metadata
        .as_ref()
        .is_some_and(|value| !value.is_object())
    {
        return Err(AppError::validation("supplement.dataset_metadata"));
    }
    let mut seen = HashSet::new();
    for (index, value) in supplement.items.iter().enumerate() {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::validation(format!("supplement.items[{index}].id")))?;
        if !is_valid_id(id) || !ids.contains(id) || !seen.insert(id) {
            return Err(AppError::validation(format!(
                "supplement.items[{index}].id"
            )));
        }
        for key in ["explanation", "example", "caution", "category"] {
            if value.get(key).is_some_and(|v| !v.is_string()) {
                return Err(AppError::validation(format!(
                    "supplement.items[{index}].{key}"
                )));
            }
        }
        for key in ["tags", "sources"] {
            if let Some(values) = value.get(key) {
                if !values
                    .as_array()
                    .is_some_and(|values| values.iter().all(Value::is_string))
                {
                    return Err(AppError::validation(format!(
                        "supplement.items[{index}].{key}"
                    )));
                }
            }
        }
        // 원문 ID를 보존하며 source_refs에는 자료·문항 ID 정규식을 적용하지 않는다.
        if let Some(refs) = value.get("source_refs") {
            if !refs.as_array().is_some_and(|refs| {
                refs.iter().all(|r| {
                    r.get("dataset_id").is_some_and(Value::is_string)
                        && r.get("item_id").is_some_and(Value::is_string)
                })
            }) {
                return Err(AppError::validation(format!(
                    "supplement.items[{index}].source_refs"
                )));
            }
        }
    }
    Ok(())
}

pub fn load_initial(root: &Path) -> AppResult<Vec<Package>> {
    let catalog: Catalog = read_json(root, "catalog.json", "catalog")?;
    if catalog.schema_version != "1.0" || catalog.datasets.is_empty() {
        return Err(AppError::validation("catalog.schema_version/datasets"));
    }
    let mut seen = HashSet::new();
    let mut packages = Vec::new();
    for (index, entry) in catalog.datasets.into_iter().enumerate() {
        if !is_valid_id(&entry.dataset_id) || !seen.insert(entry.dataset_id.clone()) {
            return Err(AppError::validation(format!(
                "catalog.datasets[{index}].dataset_id"
            )));
        }
        let learning: Learning = read_json(root, &entry.learning_file, "learning_file")?;
        if learning.dataset_id != entry.dataset_id
            || learning.title != entry.title
            || learning.items.len() != entry.question_count
        {
            return Err(AppError::validation(format!("catalog.datasets[{index}]")));
        }
        let supplement = entry
            .supplement_file
            .as_deref()
            .map(|file| read_json(root, file, "supplement_file"))
            .transpose()?;
        let package = Package {
            learning,
            supplement,
        };
        validate_package(&package)?;
        packages.push(package);
    }
    Ok(packages)
}
