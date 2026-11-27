//! Versioned import foundation. Not connected to the v1 desktop store yet.
use crate::{
    error::{AppError, AppResult},
    package::{self, Learning as LegacyLearning, Package as LegacyPackage, Supplement},
    validation::{is_unanswered, is_valid_answer, is_valid_id},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::{Component, Path},
};

pub const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CATALOG_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_ITEMS: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Criteria {
    ShortAnswer {
        answer: String,
        #[serde(default)]
        accepted_answers: Vec<String>,
    },
    Descriptive {
        reference_answer: String,
        key_points: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Item {
    ShortAnswer {
        id: String,
        question: String,
        answer: String,
        #[serde(default)]
        accepted_answers: Vec<String>,
    },
    Descriptive {
        id: String,
        question: String,
        reference_answer: String,
        key_points: Vec<String>,
    },
}

impl Item {
    pub fn id(&self) -> &str {
        match self {
            Self::ShortAnswer { id, .. } | Self::Descriptive { id, .. } => id,
        }
    }
    pub fn question(&self) -> &str {
        match self {
            Self::ShortAnswer { question, .. } | Self::Descriptive { question, .. } => question,
        }
    }
    pub fn criteria(&self) -> Criteria {
        match self {
            Self::ShortAnswer {
                answer,
                accepted_answers,
                ..
            } => Criteria::ShortAnswer {
                answer: answer.clone(),
                accepted_answers: accepted_answers.clone(),
            },
            Self::Descriptive {
                reference_answer,
                key_points,
                ..
            } => Criteria::Descriptive {
                reference_answer: reference_answer.clone(),
                key_points: key_points.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Learning {
    pub schema_version: String,
    pub dataset_id: String,
    pub title: String,
    pub items: Vec<Item>,
}

pub fn export_learning_v2(data: &Learning) -> AppResult<Vec<u8>> {
    let bytes=serde_json::to_vec(&serde_json::json!({"schema_version":"2.0","dataset_id":data.dataset_id,"title":data.title,"items":data.items}))
        .map_err(|_|AppError::validation("learning"))?;
    parse_learning(&bytes)?;
    Ok(bytes)
}

fn bounded_text(text: &str, limit: usize, field: &str) -> AppResult<()> {
    if text.len() > limit || is_unanswered(text) {
        return Err(AppError::validation(field));
    }
    Ok(())
}

pub fn validate_criteria(criteria: &Criteria) -> AppResult<()> {
    match criteria {
        Criteria::ShortAnswer {
            answer,
            accepted_answers,
        } => {
            if answer.len() > 4096
                || !is_valid_answer(answer)
                || accepted_answers.len() > 64
                || accepted_answers
                    .iter()
                    .any(|a| a.len() > 4096 || !is_valid_answer(a))
            {
                return Err(AppError::validation("criteria.short_answer"));
            }
        }
        Criteria::Descriptive {
            reference_answer,
            key_points,
        } => {
            bounded_text(reference_answer, 65_536, "criteria.reference_answer")?;
            if key_points.is_empty() || key_points.len() > 64 {
                return Err(AppError::validation("criteria.key_points"));
            }
            for point in key_points {
                bounded_text(point, 8192, "criteria.key_points")?;
            }
        }
    }
    Ok(())
}

fn json(bytes: &[u8], field: &str) -> AppResult<Value> {
    if bytes.len() > MAX_JSON_BYTES {
        return Err(AppError::validation(field));
    }
    serde_json::from_slice(bytes).map_err(|_| AppError::validation(field))
}

/// Retains strings exactly; the legacy shape is converted, never regraded here.
pub fn parse_learning(bytes: &[u8]) -> AppResult<Learning> {
    let value = json(bytes, "learning")?;
    let version = value
        .get("schema_version")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::validation("schema_version"))?;
    let data = match version {
        "1.0" => {
            if value
                .get("items")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        ["type", "reference_answer", "key_points"]
                            .iter()
                            .any(|key| item.get(*key).is_some())
                    })
                })
            {
                return Err(AppError::validation("items.version_fields"));
            }
            let legacy: LegacyLearning =
                serde_json::from_slice(bytes).map_err(|_| AppError::validation("learning"))?;
            package::validate_package(&LegacyPackage {
                learning: legacy.clone(),
                supplement: None,
            })?;
            Learning {
                schema_version: legacy.schema_version,
                dataset_id: legacy.dataset_id,
                title: legacy.title,
                items: legacy
                    .items
                    .into_iter()
                    .map(|i| Item::ShortAnswer {
                        id: i.id,
                        question: i.question,
                        answer: i.answer,
                        accepted_answers: i.accepted_answers,
                    })
                    .collect(),
            }
        }
        "2.0" => serde_json::from_slice(bytes).map_err(|_| AppError::validation("learning"))?,
        _ => {
            return Err(AppError::new(
                "UnsupportedFormat",
                "지원하지 않는 자료 버전이에요.",
                false,
            ))
        }
    };
    if !is_valid_id(&data.dataset_id) || data.items.is_empty() || data.items.len() > MAX_ITEMS {
        return Err(AppError::validation("dataset_id/items"));
    }
    bounded_text(&data.title, 4096, "title")?;
    let mut ids = HashSet::new();
    for (index, item) in data.items.iter().enumerate() {
        if !is_valid_id(item.id()) || !ids.insert(item.id()) {
            return Err(AppError::validation(format!("items[{index}].id")));
        }
        bounded_text(item.question(), 16_384, &format!("items[{index}].question"))?;
        validate_criteria(&item.criteria())
            .map_err(|_| AppError::validation(format!("items[{index}].criteria")))?;
    }
    Ok(data)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ItemKey {
    pub dataset_id: String,
    pub item_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Foundation,
    Understanding,
    Application,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningLink {
    pub item_id: String,
    pub prerequisites: Vec<ItemKey>,
    pub stage: Option<Stage>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SupplementV2 {
    schema_version: String,
    dataset_id: String,
    dataset_metadata: Option<Value>,
    items: Vec<Value>,
    #[serde(default)]
    learning_links: Vec<LearningLink>,
}
pub struct ParsedPackage {
    pub learning: Learning,
    pub supplement: Option<Supplement>,
    pub links: Vec<LearningLink>,
}

pub fn parse_package(
    learning_bytes: &[u8],
    supplement_bytes: Option<&[u8]>,
) -> AppResult<ParsedPackage> {
    let learning = parse_learning(learning_bytes)?;
    let (supplement, links) = if let Some(bytes) = supplement_bytes {
        let value = json(bytes, "supplement")?;
        let (supplement, links) = if learning.schema_version == "2.0" {
            let s: SupplementV2 =
                serde_json::from_slice(bytes).map_err(|_| AppError::validation("supplement"))?;
            (
                Supplement {
                    schema_version: s.schema_version,
                    dataset_id: s.dataset_id,
                    dataset_metadata: s.dataset_metadata,
                    items: s.items,
                },
                s.learning_links,
            )
        } else {
            if value.get("learning_links").is_some() {
                return Err(AppError::validation("supplement.learning_links.version"));
            }
            (
                serde_json::from_slice(bytes).map_err(|_| AppError::validation("supplement"))?,
                Vec::new(),
            )
        };
        let ids = learning.items.iter().map(Item::id).collect();
        package::validate_supplement(
            &supplement,
            &learning.schema_version,
            &learning.dataset_id,
            &ids,
        )?;
        if links.len() > MAX_ITEMS {
            return Err(AppError::validation("learning_links"));
        }
        for link in &links {
            if !is_valid_id(&link.item_id)
                || link.prerequisites.len() > 64
                || link
                    .prerequisites
                    .iter()
                    .any(|k| !is_valid_id(&k.dataset_id) || !is_valid_id(&k.item_id))
            {
                return Err(AppError::validation("learning_links"));
            }
        }
        (Some(supplement), links)
    } else {
        (None, Vec::new())
    };
    Ok(ParsedPackage {
        learning,
        supplement,
        links,
    })
}

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

fn read_file(root: &Path, name: &str, total: &mut usize) -> AppResult<Vec<u8>> {
    let relative = Path::new(name);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(AppError::validation("file_path"));
    }
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|_| AppError::validation("file_path"))?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(AppError::validation("file_path"));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take((MAX_JSON_BYTES + 1) as u64).read_to_end(&mut bytes))
        .map_err(|_| AppError::validation("file"))?;
    *total += bytes.len();
    if bytes.len() > MAX_JSON_BYTES || *total > MAX_CATALOG_BYTES {
        return Err(AppError::validation("file_size"));
    }
    Ok(bytes)
}

pub fn load_catalog(root: &Path) -> AppResult<Vec<ParsedPackage>> {
    let root = root
        .canonicalize()
        .map_err(|_| AppError::validation("root"))?;
    let mut total = 0;
    let bytes = read_file(&root, "catalog.json", &mut total)?;
    let value = json(&bytes, "catalog")?;
    if value.get("schema_version").and_then(Value::as_str) == Some("2.0") {
        let allowed = |v: &Value, keys: &[&str]| {
            v.as_object()
                .is_some_and(|object| object.keys().all(|k| keys.contains(&k.as_str())))
        };
        if !allowed(&value, &["schema_version", "datasets"])
            || !value
                .get("datasets")
                .and_then(Value::as_array)
                .is_some_and(|entries| {
                    entries.iter().all(|entry| {
                        allowed(
                            entry,
                            &[
                                "dataset_id",
                                "title",
                                "learning_file",
                                "supplement_file",
                                "question_count",
                            ],
                        )
                    })
                })
        {
            return Err(AppError::validation("catalog.fields"));
        }
    }
    let catalog: Catalog =
        serde_json::from_slice(&bytes).map_err(|_| AppError::validation("catalog"))?;
    if !matches!(catalog.schema_version.as_str(), "1.0" | "2.0")
        || catalog.datasets.is_empty()
        || catalog.datasets.len() > 512
    {
        return Err(AppError::validation("catalog.version/datasets"));
    }
    let mut seen = HashSet::new();
    let mut packages = Vec::new();
    for entry in catalog.datasets {
        if !is_valid_id(&entry.dataset_id) || !seen.insert(entry.dataset_id.clone()) {
            return Err(AppError::validation("catalog.dataset_id"));
        }
        let bytes = read_file(&root, &entry.learning_file, &mut total)?;
        let supplement = entry
            .supplement_file
            .map(|f| read_file(&root, &f, &mut total))
            .transpose()?;
        let package = parse_package(&bytes, supplement.as_deref())?;
        if (catalog.schema_version == "1.0" && package.learning.schema_version != "1.0")
            || entry.dataset_id != package.learning.dataset_id
            || entry.title != package.learning.title
            || entry.question_count != package.learning.items.len()
        {
            return Err(AppError::validation("catalog.dataset"));
        }
        packages.push(package);
    }
    Ok(packages)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkIssue {
    Missing,
    Duplicate,
    SelfReference,
    Cycle,
}
#[derive(Clone, Debug)]
pub struct ResolvedLink {
    pub from: ItemKey,
    pub to: ItemKey,
    pub issue: Option<LinkIssue>,
}

/// Resolves only currently installed items. Iterative SCC traversal avoids stack overflow.
pub fn resolve_links(packages: &[ParsedPackage]) -> Vec<ResolvedLink> {
    let keys: Vec<ItemKey> = packages
        .iter()
        .flat_map(|p| {
            p.learning.items.iter().map(|i| ItemKey {
                dataset_id: p.learning.dataset_id.clone(),
                item_id: i.id().into(),
            })
        })
        .collect();
    let refs = packages
        .iter()
        .flat_map(|p| {
            p.links.iter().flat_map(|link| {
                link.prerequisites.iter().map(|to| {
                    (
                        ItemKey {
                            dataset_id: p.learning.dataset_id.clone(),
                            item_id: link.item_id.clone(),
                        },
                        to.clone(),
                    )
                })
            })
        })
        .collect();
    resolve_key_links(&keys, refs)
}

pub(crate) fn resolve_key_links(
    keys: &[ItemKey],
    refs: Vec<(ItemKey, ItemKey)>,
) -> Vec<ResolvedLink> {
    let indices: HashMap<_, _> = keys
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, k)| (k, i))
        .collect();
    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    for (from, to) in refs {
        let issue = if !seen.insert((from.clone(), to.clone())) {
            Some(LinkIssue::Duplicate)
        } else if from == to {
            Some(LinkIssue::SelfReference)
        } else if !indices.contains_key(&from) || !indices.contains_key(&to) {
            Some(LinkIssue::Missing)
        } else {
            None
        };
        edges.push(ResolvedLink { from, to, issue });
    }
    let mut graph = vec![Vec::new(); keys.len()];
    let mut reverse = graph.clone();
    for e in &edges {
        if e.issue.is_none() {
            let a = indices[&e.from];
            let b = indices[&e.to];
            graph[a].push(b);
            reverse[b].push(a);
        }
    }
    let mut visited = vec![false; keys.len()];
    let mut order = Vec::new();
    for start in 0..keys.len() {
        if visited[start] {
            continue;
        }
        let mut stack = vec![(start, false)];
        while let Some((node, finish)) = stack.pop() {
            if finish {
                order.push(node);
                continue;
            }
            if visited[node] {
                continue;
            }
            visited[node] = true;
            stack.push((node, true));
            for &next in &graph[node] {
                if !visited[next] {
                    stack.push((next, false));
                }
            }
        }
    }
    let mut components = vec![usize::MAX; keys.len()];
    for start in order.into_iter().rev() {
        if components[start] != usize::MAX {
            continue;
        }
        components[start] = start;
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            for &next in &reverse[node] {
                if components[next] == usize::MAX {
                    components[next] = start;
                    stack.push(next);
                }
            }
        }
    }
    for e in &mut edges {
        if e.issue.is_none() && components[indices[&e.from]] == components[indices[&e.to]] {
            e.issue = Some(LinkIssue::Cycle);
        }
    }
    edges
}

#[cfg(test)]
mod tests;
