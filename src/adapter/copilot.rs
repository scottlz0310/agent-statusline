use crate::adapter::StatuslineAdapter;
use crate::model::ratelimit::RatelimitPayload;
use crate::model::state::StatuslineState;

pub struct CopilotAdapter;

impl StatuslineAdapter for CopilotAdapter {
    fn parse(&self, _raw_json: &str) -> (StatuslineState, Option<RatelimitPayload>) {
        (StatuslineState::default(), None)
    }
}
