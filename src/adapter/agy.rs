use crate::adapter::StatuslineAdapter;
use crate::model::ratelimit::RatelimitPayload;
use crate::model::state::StatuslineState;

pub struct AntigravityAdapter;

impl StatuslineAdapter for AntigravityAdapter {
    fn parse(&self, _raw_json: &str) -> (StatuslineState, Option<RatelimitPayload>) {
        (StatuslineState::default(), None)
    }
}
