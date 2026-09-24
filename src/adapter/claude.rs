use crate::adapter::StatuslineAdapter;
use crate::model::ratelimit::RatelimitPayload;
use crate::model::state::StatuslineState;

pub struct ClaudeAdapter;

impl StatuslineAdapter for ClaudeAdapter {
    fn parse(&self, _raw_json: &str) -> (StatuslineState, Option<RatelimitPayload>) {
        (StatuslineState::default(), None)
    }
}
