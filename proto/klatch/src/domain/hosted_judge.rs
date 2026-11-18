//! The optional hosted judge (#53, DESIGN D4). A hosted model is a third party, so it is treated like a reader of a
//! lower clearance: it only ever sees text posted at or below a ceiling label set at onboarding, and it is off
//! until someone turns it on. The label gate runs here, before the transport is touched, and fails closed: a
//! label that is missing, unknown, or above the ceiling means the text is never sent and the item is left queued
//! (`Defer`) for the rules, a local judge or the reviewer. The transport is a trait so tests use a fake and no
//! network is involved.

use super::{Judge, LabelChain, ReviewItem, Verdict};

/// How the text reaches the hosted service. An `Err` is any failure (unreachable, timeout, a reply that does not
/// parse); the judge turns it into `Defer`.
pub trait HostedTransport {
    fn screen(&self, text: &str) -> Result<Verdict, String>;
}

/// The settings for a hosted judge. The default is off, with the lowest-risk ceiling being no ceiling at all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostedConfig {
    pub enabled: bool,
    /// The highest label the hosted service may see. `None` means no label is allowed.
    pub max_label: Option<String>,
}

pub struct HostedJudge<T: HostedTransport> {
    transport: T,
    chain: LabelChain,
    config: HostedConfig,
}

impl<T: HostedTransport> HostedJudge<T> {
    pub fn new(transport: T, chain: LabelChain, config: HostedConfig) -> Self {
        Self {
            transport,
            chain,
            config,
        }
    }

    /// True only when the judge is on and the item's label is known and no higher than the ceiling.
    fn may_see(&self, item: &ReviewItem) -> bool {
        if !self.config.enabled {
            return false;
        }
        let Some(max) = self.config.max_label.as_deref() else {
            return false;
        };
        match (self.chain.rank(&item.from_label), self.chain.rank(max)) {
            (Some(held), Some(ceiling)) => held <= ceiling,
            _ => false,
        }
    }
}

impl<T: HostedTransport> Judge for HostedJudge<T> {
    fn judge(&self, item: &ReviewItem) -> Verdict {
        if !self.may_see(item) {
            return Verdict::Defer;
        }
        self.transport.screen(&item.text).unwrap_or(Verdict::Defer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Records every text it is asked to send.
    struct Fake {
        sent: RefCell<Vec<String>>,
        reply: Result<Verdict, String>,
    }
    impl Fake {
        fn saying(reply: Result<Verdict, String>) -> Self {
            Self {
                sent: RefCell::new(Vec::new()),
                reply,
            }
        }
    }
    impl HostedTransport for &Fake {
        fn screen(&self, text: &str) -> Result<Verdict, String> {
            self.sent.borrow_mut().push(text.to_string());
            self.reply.clone()
        }
    }

    fn item(label: &str) -> ReviewItem {
        ReviewItem {
            id: 1,
            room: "r".into(),
            seq: 1,
            to_label: "public".into(),
            from_label: label.into(),
            reason: "crossing".into(),
            text: "the text".into(),
        }
    }

    fn on(max: Option<&str>) -> HostedConfig {
        HostedConfig {
            enabled: true,
            max_label: max.map(String::from),
        }
    }

    fn judge(f: &Fake, config: HostedConfig) -> HostedJudge<&Fake> {
        HostedJudge::new(f, LabelChain::default(), config)
    }

    #[test]
    fn it_is_off_by_default_and_sends_nothing() {
        let f = Fake::saying(Ok(Verdict::Allow));
        let j = judge(&f, HostedConfig::default());
        assert_eq!(j.judge(&item("public")), Verdict::Defer);
        assert!(f.sent.borrow().is_empty());
    }

    #[test]
    fn content_above_the_ceiling_is_never_sent() {
        let f = Fake::saying(Ok(Verdict::Allow));
        let j = judge(&f, on(Some("internal")));
        for label in ["private", "secret"] {
            assert_eq!(j.judge(&item(label)), Verdict::Defer, "{label}");
        }
        assert!(f.sent.borrow().is_empty(), "{:?}", f.sent.borrow());
    }

    #[test]
    fn content_at_or_below_the_ceiling_is_sent_when_enabled() {
        let f = Fake::saying(Ok(Verdict::Allow));
        let j = judge(&f, on(Some("internal")));
        assert_eq!(j.judge(&item("internal")), Verdict::Allow);
        assert_eq!(j.judge(&item("public")), Verdict::Allow);
        assert_eq!(f.sent.borrow().len(), 2);
    }

    #[test]
    fn any_doubt_about_a_label_fails_closed() {
        let f = Fake::saying(Ok(Verdict::Allow));
        for config in [on(None), on(Some("nonsense")), on(Some(""))] {
            let j = judge(&f, config);
            assert_eq!(j.judge(&item("public")), Verdict::Defer);
        }
        let j = judge(&f, on(Some("internal")));
        for label in ["", "unknown", "PUBLIC", "public "] {
            assert_eq!(j.judge(&item(label)), Verdict::Defer, "{label:?}");
        }
        assert!(f.sent.borrow().is_empty());
    }

    #[test]
    fn a_transport_failure_leaves_the_item_queued() {
        let f = Fake::saying(Err("timeout".into()));
        let j = judge(&f, on(Some("internal")));
        assert_eq!(j.judge(&item("public")), Verdict::Defer);
        assert_eq!(f.sent.borrow().len(), 1);
    }

    #[test]
    fn a_custom_chain_sets_its_own_order() {
        let f = Fake::saying(Ok(Verdict::Allow));
        let chain = LabelChain::new(&["low", "mid", "high"]).unwrap();
        let j = HostedJudge::new(&f, chain, on(Some("mid")));
        assert_eq!(j.judge(&item("mid")), Verdict::Allow);
        assert_eq!(j.judge(&item("high")), Verdict::Defer);
        assert_eq!(f.sent.borrow().len(), 1);
    }
}
