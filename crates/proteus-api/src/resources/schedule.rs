use chrono::Utc;
use proteus_core::{next_run_after, validate_schedule};
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulePreviewRequest {
    pub schedule: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulePreviewResponse {
    pub schedule: String,
    pub next_run_at: String,
}

pub fn preview_schedule(req: SchedulePreviewRequest) -> ApiResult<SchedulePreviewResponse> {
    let schedule = req.schedule.trim().to_string();
    if schedule.is_empty() {
        return Err(ApiError::BadRequest(
            "schedule must not be empty".to_string(),
        ));
    }
    validate_schedule(&schedule).map_err(ApiError::BadRequest)?;
    let next = next_run_after(&schedule, Utc::now()).map_err(ApiError::BadRequest)?;
    Ok(SchedulePreviewResponse {
        schedule,
        next_run_at: next.to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_daily() {
        let res = preview_schedule(SchedulePreviewRequest {
            schedule: "0 2 * * *".into(),
        })
        .expect("ok");
        assert!(res.next_run_at.contains('T'));
    }

    #[test]
    fn preview_rejects_garbage() {
        assert!(preview_schedule(SchedulePreviewRequest {
            schedule: "nope".into(),
        })
        .is_err());
    }
}
