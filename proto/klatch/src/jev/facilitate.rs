//! The screened routing advice (#917): one request that asks the route question and the noul screen together
//! (patterns__noul-screen.md, fan-out), then applies [`pick_screened`].
//!
//! This is the pure core the facilitator loop will call. It holds the guard-first rule in one place: the candidates are
//! the ones the rules and the guard already allow, every fact is label gated before it is sent ([`route_question`]),
//! and the answer can only narrow (to nothing, or back to the rules decision) or pick among those candidates, never
//! widen. Every failure (a gated sender or topic, no key, a refused request, a transport error, an unparsable reply, a
//! missing screen answer) leaves the rules decision. What still has to exist before the loop calls this is a per-fact
//! label source and the operator's thresholds (ticket #917); nothing here reads either.

use super::{
    Band, JevClient, JevTransport, Labelled, NoulThresholds, Question, ROUTE_QUESTION_ID,
    pick_screened, route_question,
};
use crate::domain::LabelChain;

/// The id of the screen question in the shared request.
pub const SCREEN_QUESTION_ID: &str = "needs_route";

/// The fixed instruction of the screen: does this ask need a routed recipient at all.
const SCREEN_INSTRUCTIONS: &str = "Does this ask need to be routed to a recipient at all?";

/// Advise the route for one ask. `rules` is what the rules router decided (`None` for nothing); the result is the final
/// decision. The confidence threshold is the client's configured one; `thresholds` are the screen's yes and no bands.
#[allow(clippy::too_many_arguments)]
pub fn advise_route<T: JevTransport>(
    client: &JevClient<T>,
    thresholds: NoulThresholds,
    chain: &LabelChain,
    ceiling: Option<&str>,
    sender: &Labelled,
    topic: &Labelled,
    candidates: &[Labelled],
    rules: Option<&str>,
) -> Option<String> {
    let keep_rules = || rules.map(String::from);
    let Some((route, offered)) = route_question(chain, ceiling, sender, topic, candidates) else {
        return keep_rules();
    };
    let screen = Question::noul(SCREEN_QUESTION_ID, SCREEN_INSTRUCTIONS);
    let Ok(answers) = client.ask(&route.state, &[route.question, screen]) else {
        return keep_rules();
    };
    let (Some(choice), Some(p)) = (
        answers.choice(ROUTE_QUESTION_ID),
        answers.noul(SCREEN_QUESTION_ID),
    ) else {
        return keep_rules();
    };
    let band: Band = thresholds.band(p);
    pick_screened(&offered, rules, band, choice, client.config().threshold())
}

#[cfg(test)]
mod tests;
