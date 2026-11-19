//! Jev as a graded judge (#914): the production path, `HostedJudge<JevJudge<_>>`, with a transport wrapper that
//! keeps the raw reply so the runner can also read Jev's own pick, confidence and token usage. The verdict is the
//! production one, threshold and label gate included.
//!
//! The real HTTP transport is #871 and is not merged, so the command line reports "transport not available" for
//! `--judge jev`; this type is ready for it (pass the transport to [`JevEval::new`]) and is tested with a fake.

use crate::judge_eval::corpus::Truth;
use crate::judge_eval::judges::{EvalJudge, Judged};
use klatch::domain::{
    HostedConfig, HostedJudge, Judge, LabelChain, ReviewItem, Verdict, label_below_ceiling,
};
use klatch::jev::{
    ApiKey, JUDGE_QUESTION_ID, JevClient, JevConfig, JevJudge, JevTransport, JevTransportError,
    Question, parse_response,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Passes every call through and keeps the last successful reply body.
struct Recording<T> {
    inner: T,
    last: Rc<RefCell<Option<String>>>,
}

impl<T: JevTransport> JevTransport for Recording<T> {
    fn post(&self, key: &ApiKey, body: &str) -> Result<String, JevTransportError> {
        let r = self.inner.post(key, body);
        if let Ok(reply) = &r {
            *self.last.borrow_mut() = Some(reply.clone());
        }
        r
    }
}

pub struct JevEval<T: JevTransport> {
    judge: HostedJudge<JevJudge<Recording<T>>>,
    last: Rc<RefCell<Option<String>>>,
    model: String,
    ceiling: String,
}

impl<T: JevTransport> JevEval<T> {
    /// `key` yields the API key at each call (production reads `TYPESAFE_API_KEY`).
    pub fn new(
        transport: T,
        config: JevConfig,
        ceiling: &str,
        key: Box<dyn Fn() -> Option<String>>,
    ) -> Self {
        let last = Rc::new(RefCell::new(None));
        let model = config.model().to_string();
        let client = JevClient::new(
            Recording {
                inner: transport,
                last: last.clone(),
            },
            config,
        )
        .with_key_source(key);
        let judge = HostedJudge::new(
            JevJudge::new(client),
            LabelChain::default(),
            HostedConfig {
                enabled: true,
                max_label: Some(ceiling.to_string()),
            },
        );
        JevEval {
            judge,
            last,
            model,
            ceiling: ceiling.to_string(),
        }
    }
}

impl<T: JevTransport> EvalJudge for JevEval<T> {
    fn name(&self) -> String {
        format!("jev:{}", self.model)
    }

    fn evaluate(&self, item: &ReviewItem) -> Judged {
        self.last.borrow_mut().take();
        let verdict = self.judge.judge(item);
        let reply = self.last.borrow_mut().take();
        let raw = reply.as_deref().and_then(|r| {
            let answers = parse_response(r, &self.model, &[Question::judge()]).ok()?;
            let a = answers.choice(JUDGE_QUESTION_ID)?;
            Some((Truth::parse(&a.choice)?, a.confidence))
        });
        let usage = reply
            .as_deref()
            .and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok())
            .map(|v| v["usage"].clone())
            .unwrap_or_default();
        // a text the label gate keeps back is not a failed call
        let sent = label_below_ceiling(
            &LabelChain::default(),
            Some(&self.ceiling),
            &item.from_label,
        );
        let error = (sent && verdict == Verdict::Defer && raw.is_none()).then(|| {
            "no usable answer (no key, unreachable, or a reply that did not parse)".to_string()
        });
        Judged {
            verdict,
            raw,
            tokens_in: usage["input_tokens"].as_u64(),
            tokens_out: usage["output_tokens"].as_u64(),
            cost_usd: None,
            error,
        }
    }
}

#[cfg(test)]
#[path = "jev_tests.rs"]
mod tests;
