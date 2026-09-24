pub mod agy;
pub mod claude;
pub mod copilot;

use crate::model::ratelimit::RatelimitPayload;
use crate::model::state::StatuslineState;

pub trait StatuslineAdapter {
    fn parse(&self, raw_json: &str) -> (StatuslineState, Option<RatelimitPayload>);
}
