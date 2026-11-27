use super::*;
use crate::{exam::validate_exam_id, learning_format::validate_criteria, validation::is_valid_id};
use std::collections::HashMap;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EditMaterialRequest {
    pub request_id: String,
    pub dataset_id: String,
    pub item_id: String,
    pub expected_revision: u64,
    pub criteria: Criteria,
    pub explanation: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct EditMaterialResult {
    pub dataset_id: String,
    pub item_id: String,
    pub dataset_revision: u64,
    pub changed: bool,
}
#[derive(Default)]
pub struct MaterialEditState {
    receipts: HashMap<String, (String, EditMaterialResult)>,
}
fn conflict() -> AppError {
    AppError::new(
        "MaterialConflict",
        "자료가 바뀌었어요. 편집을 취소하고 최신 자료를 불러온 뒤 다시 수정해주세요.",
        false,
    )
}
impl MaterialEditState {
    pub fn save(
        &mut self,
        db: &mut Connection,
        version: u8,
        request: EditMaterialRequest,
    ) -> AppResult<EditMaterialResult> {
        if !matches!(version, 1 | 2)
            || !is_valid_id(&request.dataset_id)
            || !is_valid_id(&request.item_id)
            || request.expected_revision == 0
            || request.expected_revision >= 9_007_199_254_740_991
        {
            return Err(AppError::validation("material_id/revision"));
        }
        validate_exam_id(&request.request_id).map_err(|_| AppError::validation("request_id"))?;
        validate_criteria(&request.criteria).map_err(|mut e| {
            e.message = match request.criteria {
                Criteria::ShortAnswer { .. } => "정답과 허용 표기는 각각 4단어 이하로 입력해주세요. 앞뒤 공백·줄바꿈·빈 답은 사용할 수 없어요.",
                Criteria::Descriptive { .. } => "참고 답안과 핵심 내용을 비우지 말고 입력 길이와 항목 수를 확인해주세요.",
            }.into();
            e.field_path = None;
            e
        })?;
        if request
            .explanation
            .as_ref()
            .is_some_and(|s| s.len() > 65_536)
        {
            return Err(AppError::new(
                "ValidationError",
                "해설은 UTF-8 기준 64 KiB 이하로 입력해주세요.",
                false,
            ));
        }
        let digest = format!("{:x}", Sha256::digest(encode(&request)?.as_bytes()));
        if let Some((previous, result)) = self.receipts.get(&request.request_id) {
            return if previous == &digest {
                Ok(result.clone())
            } else {
                Err(AppError::new("RequestConflict","같은 저장 요청에 다른 내용을 적용할 수 없어요. 최신 자료에서 다시 시도해주세요.",false))
            };
        }
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rev = revision(&tx, &request.dataset_id)?.ok_or_else(conflict)?;
        if rev < 1 || rev as u64 != request.expected_revision {
            return Err(conflict());
        }
        let mut data = read_snapshot(&tx, &request.dataset_id, version)?.ok_or_else(conflict)?;
        let before = encode(&data)?;
        let item = data
            .items
            .iter_mut()
            .find(|i| i.id == request.item_id)
            .ok_or_else(conflict)?;
        if std::mem::discriminant(&item.criteria) != std::mem::discriminant(&request.criteria) {
            return Err(AppError::new(
                "ValidationError",
                "이 화면에서는 문항 유형을 바꿀 수 없어요.",
                false,
            ));
        }
        item.criteria = request.criteria.clone();
        let mut extra = data
            .supplements
            .get(&request.item_id)
            .map(|v| v.as_object().cloned().ok_or_else(AppError::inconsistent))
            .transpose()?
            .unwrap_or_default();
        match &request.explanation {
            Some(text) => {
                extra.insert("explanation".into(), Value::String(text.clone()));
            }
            None => {
                extra.remove("explanation");
            }
        }
        // Existing non-explanation metadata, including original source_refs, is retained.
        if !extra.is_empty() || data.supplements.contains_key(&request.item_id) {
            data.supplements
                .insert(request.item_id.clone(), Value::Object(extra));
        }
        let after = encode(&data)?;
        let changed = before != after;
        let result = EditMaterialResult {
            dataset_id: request.dataset_id.clone(),
            item_id: request.item_id.clone(),
            dataset_revision: request.expected_revision + u64::from(changed),
            changed,
        };
        if changed {
            if version == 2 {
                tx.execute("UPDATE items SET criteria_json=?3 WHERE dataset_id=?1 AND item_id=?2 AND is_active=1",params![request.dataset_id,request.item_id,encode(&request.criteria)?])?;
            } else if let Criteria::ShortAnswer {
                answer,
                accepted_answers,
            } = &request.criteria
            {
                tx.execute("UPDATE items SET answer=?3,accepted_answers_json=?4 WHERE dataset_id=?1 AND item_id=?2 AND is_active=1",params![request.dataset_id,request.item_id,answer,encode(accepted_answers)?])?;
            }
            if let Some(payload) = data.supplements.get(&request.item_id) {
                tx.execute("INSERT INTO supplements VALUES (?1,?2,?3) ON CONFLICT(dataset_id,item_id) DO UPDATE SET payload_json=excluded.payload_json",params![request.dataset_id,request.item_id,encode(payload)?])?;
            }
            let hash = format!("{:x}", Sha256::digest(after.as_bytes()));
            tx.execute(
                "UPDATE datasets SET revision=?2,content_hash=?3 WHERE dataset_id=?1",
                params![request.dataset_id, result.dataset_revision as i64, hash],
            )?;
        }
        tx.commit()?;
        self.receipts
            .insert(request.request_id, (digest, result.clone()));
        Ok(result)
    }
}
#[cfg(test)]
mod tests;
