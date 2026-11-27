use crate::{
    error::{AppError, AppResult},
    validation::{is_unanswered, is_valid_id},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const RULE: &str = "exact-v1";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct StartExamRequest {
    pub exam_id: String,
    pub dataset_ids: Vec<String>,
    pub count_mode: String,
    pub count: Option<u32>,
    pub order_mode: String,
    pub review_mode: String,
}

impl StartExamRequest {
    pub fn validate(&self) -> AppResult<()> {
        validate_exam_id(&self.exam_id)?;
        validate_scope(&self.dataset_ids, &self.review_mode)?;
        if !matches!(self.order_mode.as_str(), "ordered" | "random") {
            return Err(AppError::validation("order_mode/review_mode"));
        }
        if !matches!(
            (self.count_mode.as_str(), self.count),
            ("all", None) | ("count", Some(1..))
        ) {
            return Err(AppError::validation("count_mode/count"));
        }
        Ok(())
    }
}

pub fn validate_scope(dataset_ids: &[String], review_mode: &str) -> AppResult<()> {
    let mut ids = std::collections::HashSet::new();
    if dataset_ids.is_empty()
        || dataset_ids
            .iter()
            .any(|id| !is_valid_id(id) || !ids.insert(id))
    {
        return Err(AppError::validation("dataset_ids"));
    }
    if !matches!(review_mode, "general" | "wrong_first" | "wrong_only") {
        return Err(AppError::validation("review_mode"));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct CandidateRequest {
    pub dataset_ids: Vec<String>,
    pub review_mode: String,
}
#[derive(Serialize)]
pub struct CandidatePreview {
    pub candidate_count: u32,
    pub total_count: u32,
    pub review_target_count: u32,
}
#[derive(Serialize)]
pub struct ExamHistoryRow {
    pub assessment_counts: Option<crate::assessment::AssessmentCounts>,
    pub assessment_status: Option<String>,
    pub exam_id: String,
    pub status: String,
    pub dataset_titles: Vec<String>,
    pub question_count: usize,
    pub answered_count: usize,
    pub current_position: u32,
    pub saved_at: String,
    pub completed_at: Option<String>,
    pub completion_order: Option<i64>,
    pub correct_count: Option<usize>,
    /// 이 기록만 읽지 못했을 때의 안내. 다른 기록의 조회는 계속한다.
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordEntry<T> {
    Exam {
        row_key: String,
        #[serde(flatten)]
        record: T,
    },
    Error {
        row_key: String,
        error: String,
    },
}

impl<T> RecordEntry<T> {
    pub fn record(&self) -> Option<&T> {
        match self {
            Self::Exam { record, .. } => Some(record),
            Self::Error { .. } => None,
        }
    }
}

/// 후보의 원래 인덱스는 선택한 묶음 순서와 문항 순서다. 난수 공급원은 검사에서 주입한다.
pub fn select_indices(
    targets: &[bool],
    mode: &str,
    order: &str,
    count: Option<u32>,
    random: &mut impl FnMut(&mut [usize]),
) -> AppResult<Vec<usize>> {
    if !matches!(mode, "general" | "wrong_first" | "wrong_only")
        || !matches!(order, "ordered" | "random")
    {
        return Err(AppError::validation("review_mode/order_mode"));
    }
    let mut eligible: Vec<usize> = (0..targets.len())
        .filter(|i| mode != "wrong_only" || targets[*i])
        .collect();
    if eligible.is_empty() {
        return Err(AppError::new(
            "NoCandidates",
            if mode == "wrong_only" {
                "선택한 범위에 현재 오답 대상이 없어요. 다른 모드나 자료를 선택해주세요."
            } else {
                "선택한 범위에 출제할 문항이 없어요."
            },
            false,
        ));
    }
    let amount = count.map(|n| n as usize).unwrap_or(eligible.len());
    if amount == 0 || amount > eligible.len() {
        return Err(AppError::new(
            "InvalidCount",
            format!(
                "현재 조건에서 최대 {}문항을 출제할 수 있어요.",
                eligible.len()
            ),
            false,
        ));
    }
    if mode == "wrong_first" {
        let mut wrong: Vec<usize> = eligible.iter().copied().filter(|i| targets[*i]).collect();
        let mut rest: Vec<usize> = eligible.iter().copied().filter(|i| !targets[*i]).collect();
        if order == "random" {
            random(&mut wrong);
            random(&mut rest);
        }
        wrong.extend(rest);
        eligible = wrong;
    } else if order == "random" {
        random(&mut eligible);
    }
    eligible.truncate(amount);
    if order == "ordered" {
        eligible.sort_unstable();
    } else if mode == "wrong_first" {
        random(&mut eligible);
    }
    Ok(eligible)
}

pub fn validate_exam_id(value: &str) -> AppResult<()> {
    let id = Uuid::parse_str(value).map_err(|_| AppError::validation("exam_id"))?;
    if id.get_version_num() != 4
        || id.get_variant() != uuid::Variant::RFC4122
        || id.to_string() != value
    {
        return Err(AppError::validation("exam_id"));
    }
    Ok(())
}

#[derive(Clone, Deserialize)]
pub struct SubmitExamRequest {
    pub exam_id: String,
    pub revision: u64,
    pub current_position: u32,
    pub responses: Vec<String>,
}

#[derive(Serialize)]
pub struct SaveProgressResult {
    pub outcome: &'static str,
    pub revision: u64,
    pub status: String,
}

#[derive(Serialize)]
pub struct PendingExam {
    pub exam_id: String,
    pub status: String,
    pub dataset_titles: Vec<String>,
    pub question_count: usize,
    pub answered_count: usize,
    pub current_position: u32,
    pub saved_at: String,
    /// 이 시험만 읽지 못했을 때의 안내. 다른 시험의 조회는 계속한다.
    pub error: Option<String>,
}

impl SubmitExamRequest {
    pub fn validate(&self, count: usize) -> AppResult<()> {
        if self.responses.len() != count
            || self.current_position as usize >= count
            || self.revision > MAX_SAFE_INTEGER
        {
            return Err(AppError::validation("responses/current_position/revision"));
        }
        Ok(())
    }
}

#[derive(Clone, Serialize)]
pub struct SessionItem {
    pub item_type: String,
    pub position: u32,
    pub dataset_id: String,
    pub item_id: String,
    pub question: String,
    pub response_text: String,
}

#[derive(Clone, Serialize)]
pub struct ExamSession {
    pub db_version: u8,
    pub exam_id: String,
    pub status: String,
    pub revision: u64,
    pub current_position: u32,
    pub settings: StartExamRequest,
    pub dataset_titles: Vec<String>,
    pub items: Vec<SessionItem>,
}

#[derive(Serialize)]
pub struct ResultItem {
    pub position: u32,
    pub question: String,
    pub response_text: String,
    pub answer: String,
    pub accepted_answers: Vec<String>,
    pub judgement: String,
    pub incorrect_reason: Option<String>,
    pub is_review_target: bool,
    pub correct_streak: u8,
    pub supplement: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct ExamResult {
    pub exam_id: String,
    pub completed_at: String,
    pub completion_order: i64,
    pub settings: StartExamRequest,
    pub dataset_titles: Vec<String>,
    pub correct_count: usize,
    pub incorrect_count: usize,
    pub unanswered_count: usize,
    pub items: Vec<ResultItem>,
}

pub fn grade(
    response: &str,
    answer: &str,
    aliases: &[String],
) -> (&'static str, Option<&'static str>) {
    if is_unanswered(response) {
        ("incorrect", Some("unanswered"))
    } else if response == answer || aliases.iter().any(|alias| alias == response) {
        ("correct", None)
    } else {
        ("incorrect", Some("no_match"))
    }
}

// UUID의 첫 32비트는 버전·variant 비트를 포함하지 않는 OS 난수다.
// 나머지 연산의 편향을 막고 Fisher–Yates로 중복 없이 섞는다.
pub fn shuffle<T>(items: &mut [T]) {
    for index in (1..items.len()).rev() {
        let bound = (index + 1) as u32;
        let ceiling = u32::MAX - u32::MAX % bound;
        let selected = loop {
            let id = Uuid::new_v4();
            let value = u32::from_le_bytes(id.as_bytes()[..4].try_into().unwrap());
            if value < ceiling {
                break (value % bound) as usize;
            }
        };
        items.swap(index, selected);
    }
}
