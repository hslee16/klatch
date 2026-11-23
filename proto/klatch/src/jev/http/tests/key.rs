use super::*;

#[test]
fn a_key_that_could_split_a_header_is_refused_and_nothing_is_sent() {
    let s = Server::start(|_| http("200 OK", &[], "{}"));
    for bad in ["abc\r\nX-Evil: 1", "abc\ndef", "abc\u{0}def"] {
        let k = ApiKey::new(bad.to_string());
        assert_eq!(
            loopback(&s).send(&k, &JevRequest::models()).unwrap_err(),
            JevTransportError::Network
        );
    }
    assert_eq!(s.connections(), 0);
}

struct Instant0;
impl Clock for Instant0 {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}
struct Counting(Arc<Mutex<Vec<Duration>>>);
impl Sleeper for Counting {
    fn sleep(&self, d: Duration) {
        self.0.lock().unwrap().push(d);
    }
}

#[test]
fn a_429_then_a_200_succeeds_through_the_retry_policy() {
    let ok = fixtures::LIVE_NOUL.to_string();
    let s = Server::start(move |n| {
        if n == 0 {
            http("429 Too Many Requests", &["retry-after-ms: 20"], "{}")
        } else {
            http("200 OK", &[], &ok)
        }
    });
    let slept = Arc::new(Mutex::new(vec![]));
    let client = JevClient::new(loopback(&s), JevConfig::new("jev-1.14.0", 0.8).unwrap())
        .with_key_source(Box::new(|| Some(KEY.to_string())))
        .with_gate(crate::jev::Gate::new(Default::default(), Some("internal")))
        .with_retry(
            RetryPolicy::default(),
            Box::new(Instant0),
            Box::new(Counting(slept.clone())),
            Box::new(NoJitter),
        );
    let q = Question::noul("billing", "is it about billing?");
    let a = client
        .ask(&crate::jev::Entry::text("public", "hello"), &[q])
        .unwrap();
    assert!(a.noul("billing").is_some());
    assert_eq!(s.connections(), 2);
    assert_eq!(*slept.lock().unwrap(), vec![Duration::from_millis(20)]);
}

#[test]
fn a_401_is_not_retried_and_a_persistent_429_stops_at_the_policy() {
    let s = Server::start(|_| http("401 Unauthorized", &[], fixtures::ERR_AUTH));
    let slept = Arc::new(Mutex::new(vec![]));
    let mk = |s: &Server| {
        JevClient::new(loopback(s), JevConfig::new("jev-1.14.0", 0.8).unwrap())
            .with_key_source(Box::new(|| Some(KEY.to_string())))
            .with_retry(
                RetryPolicy::default(),
                Box::new(Instant0),
                Box::new(Counting(slept.clone())),
                Box::new(NoJitter),
            )
    };
    assert!(matches!(
        mk(&s).list_models(),
        Err(JevError::Unauthorized { .. })
    ));
    assert_eq!(s.connections(), 1);
    let s = Server::start(|_| http("429 Too Many Requests", &["retry-after-ms: 1"], "{}"));
    assert!(matches!(mk(&s).list_models(), Err(JevError::RateLimited)));
    assert_eq!(s.connections(), 3, "one try and two retries");
}

#[test]
fn the_key_is_in_no_error_display_or_debug() {
    let mut texts = vec![];
    let echo = format!(r#"{{"detail":[{{"input":"{KEY}"}}]}}"#);
    for reply in [
        http(
            "401 Unauthorized",
            &[],
            &format!(
                r#"{{"detail":{{"error_type":"authentication_error","message":"bad {KEY}"}}}}"#
            ),
        ),
        http("422 Unprocessable Entity", &[], &echo),
        http("500 Oops", &[], KEY),
    ] {
        let cell = Mutex::new(Some(reply));
        let s = Server::start(move |_| cell.lock().unwrap().take().unwrap_or(Reply::Hang));
        let e = loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err();
        texts.push(format!("{e:?}"));
        let client = JevClient::new(loopback(&s), JevConfig::new("jev-1.14.0", 0.8).unwrap())
            .with_key_source(Box::new(|| Some(KEY.to_string())));
        if let Err(e) = client.list_models() {
            texts.push(format!("{e:?} {e}"));
        }
    }
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let e = HttpTransport::loopback_for_tests(closed, false, opts())
        .send(&key(), &JevRequest::models())
        .unwrap_err();
    texts.push(format!("{e:?}"));
    texts.push(format!(
        "{:?}",
        HttpTransport::with_base("https://api.typesafe.ai", opts()).unwrap()
    ));
    texts.push(format!("{:?}", key()));
    for t in texts {
        assert!(!t.contains(KEY), "the key leaked: {t}");
    }
}

#[test]
fn the_real_key_comes_from_the_environment_through_the_client_at_call_time() {
    // with no key in the lookup the client refuses before the transport is touched
    let s = Server::start(|_| http("200 OK", &[], "{}"));
    let client = JevClient::new(loopback(&s), JevConfig::new("jev-1.14.0", 0.8).unwrap())
        .with_key_source(Box::new(|| None));
    assert!(matches!(client.list_models(), Err(JevError::NoKey)));
    assert_eq!(s.connections(), 0);
}
