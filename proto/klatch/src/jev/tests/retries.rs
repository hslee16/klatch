use super::*;

// ---- errors through the client: status and error_type only, never a body ----

fn echoing_422() -> String {
    fixtures::echoing_422(TEXT)
}

#[test]
fn every_documented_status_is_typed_and_shows_no_body() {
    let cases = [
        (
            400,
            JevError::BadRequest {
                error_type: Some(ErrorType::ApiUsage),
            },
            fixtures::ERR_INVALID.to_string(),
        ),
        (
            401,
            JevError::Unauthorized {
                error_type: Some(ErrorType::Authentication),
            },
            fixtures::ERR_AUTH.to_string(),
        ),
        (
            403,
            JevError::Forbidden {
                error_type: Some(ErrorType::Authentication),
            },
            fixtures::ERR_AUTH.to_string(),
        ),
        (413, JevError::PayloadTooLarge, echoing_422()),
        (415, JevError::UnsupportedMediaType, echoing_422()),
        (422, JevError::Unprocessable, echoing_422()),
        (429, JevError::RateLimited, echoing_422()),
        (529, JevError::Overloaded, echoing_422()),
    ];
    for (status, want, body) in cases {
        let f = FakeTransport::status(status, &body);
        let err = ask_judge(&f).unwrap_err();
        assert_eq!(err, want, "{status}");
        assert_eq!(err.status(), Some(status));
        let shown = format!("{err} {err:?}");
        assert!(shown.contains(&status.to_string()));
        for banned in [TEXT, "tuesday", SECRET, "Field required"] {
            assert!(!shown.contains(banned), "{status}: {shown}");
        }
    }
}

#[test]
fn the_context_error_is_typed_and_never_retried() {
    let f = FakeTransport::status(400, fixtures::ERR_CONTEXT);
    let (c, _) = retrying(&f);
    assert_eq!(
        c.ask(&state(), &[judge_question()]).unwrap_err(),
        JevError::ContextExceeded
    );
    assert_eq!(f.calls(), 1);
}

// ---- retry through the client, on an injected clock ----

/// A clock that moves only when the sleeper sleeps; both share the same cell, and every sleep is recorded.
struct Clk(Shared);
#[derive(Clone)]
struct Shared(Rc<(Cell<Duration>, RefCell<Vec<Duration>>)>);
impl Clk {
    fn new() -> (Clk, Shared) {
        let sh = Shared(Rc::new((Cell::new(Duration::ZERO), RefCell::new(vec![]))));
        (Clk(sh.clone()), sh)
    }
}
impl Clock for Clk {
    fn now(&self) -> Duration {
        (self.0).0.0.get()
    }
}
struct Slp(Shared);
impl Sleeper for Slp {
    fn sleep(&self, d: Duration) {
        (self.0).0.1.borrow_mut().push(d);
        (self.0).0.0.set((self.0).0.0.get() + d);
    }
}

fn retrying(f: &FakeTransport) -> (JevClient<&FakeTransport>, Shared) {
    let (clk, sh) = Clk::new();
    let c = client(f).with_retry(
        RetryPolicy::default(),
        Box::new(clk),
        Box::new(Slp(sh.clone())),
        Box::new(retry::NoJitter),
    );
    (c, sh)
}

#[test]
fn a_429_then_a_success_is_retried_after_the_backoff_without_waiting() {
    let f = FakeTransport::replaying(vec![
        Err(JevTransportError::status(429, &echoing_422())),
        Err(status_with_headers(529, None, Some("250"))),
        Ok(fixtures::LIVE_CHOICE.to_string()),
    ]);
    let (c, sh) = retrying(&f);
    assert!(c.ask(&state(), &[judge_question()]).is_ok());
    assert_eq!(f.calls(), 3);
    assert_eq!(
        *(sh.0).1.borrow(),
        vec![Duration::from_millis(500), Duration::from_millis(250)]
    );
}

#[test]
fn the_retries_run_out_and_the_last_status_is_the_error() {
    let f = FakeTransport::status(529, "");
    let (c, sh) = retrying(&f);
    assert_eq!(
        c.ask(&state(), &[judge_question()]).unwrap_err(),
        JevError::Overloaded
    );
    assert_eq!(f.calls(), 3, "one try and two retries");
    assert_eq!((sh.0).1.borrow().len(), 2);
}

#[test]
fn the_other_statuses_are_never_retried() {
    for status in [400, 401, 403, 413, 415, 422, 500, 502, 503, 504] {
        let f = FakeTransport::status(status, "");
        let (c, sh) = retrying(&f);
        assert!(c.ask(&state(), &[judge_question()]).is_err());
        assert_eq!(f.calls(), 1, "{status}");
        assert!(sh.0.1.borrow().is_empty());
    }
    let f = FakeTransport::saying(Err(JevTransportError::Network));
    let (c, _) = retrying(&f);
    assert_eq!(
        c.ask(&state(), &[judge_question()]).unwrap_err(),
        JevError::Unreachable
    );
    assert_eq!(f.calls(), 1);
}

#[test]
fn a_client_with_no_retry_policy_tries_once() {
    let f = FakeTransport::status(429, "");
    assert_eq!(ask_judge(&f).unwrap_err(), JevError::RateLimited);
    assert_eq!(f.calls(), 1);
}

#[test]
fn a_judge_behind_a_busy_api_retries_then_defers() {
    let f = FakeTransport::status(429, "");
    let (c, _) = retrying(&f);
    let j = HostedJudge::new(
        JevJudge::new(c),
        LabelChain::default(),
        HostedConfig {
            enabled: true,
            max_label: Some("internal".into()),
        },
    );
    assert_eq!(j.judge(&item("public", "hi")), Verdict::Defer);
    assert_eq!(f.calls(), 3);
}
