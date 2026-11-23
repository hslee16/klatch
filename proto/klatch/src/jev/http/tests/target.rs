use super::*;

#[test]
fn the_target_is_https_and_the_one_host_and_nothing_else() {
    assert!(Target::parse("https://api.typesafe.ai").is_ok());
    assert!(Target::parse("https://api.typesafe.ai/").is_ok());
    assert!(Target::parse("https://API.TYPESAFE.AI").is_ok());
    assert_eq!(Target::production().base(), crate::jev::transport::BASE_URL);
    for (url, why) in [
        ("http://api.typesafe.ai", TargetError::NotHttps),
        ("ftp://api.typesafe.ai", TargetError::NotHttps),
        ("api.typesafe.ai", TargetError::NotHttps),
        ("https://evil.example", TargetError::WrongHost),
        (
            "https://api.typesafe.ai.evil.example",
            TargetError::WrongHost,
        ),
        (
            "https://evil.example/api.typesafe.ai",
            TargetError::WrongHost,
        ),
        (
            "https://api.typesafe.ai@evil.example",
            TargetError::WrongHost,
        ),
        (
            "https://evil.example@api.typesafe.ai",
            TargetError::Userinfo,
        ),
        ("https://api.typesafe.ai:8443", TargetError::Port),
        ("https://api.typesafe.ai:443", TargetError::Port),
        ("https://api.typesafe.ai/v1", TargetError::Path),
        ("https://api.typesafe.ai?x=1", TargetError::Path),
        ("https://api.typesafe.ai#f", TargetError::Path),
        ("https://api.typesafe.ai.", TargetError::WrongHost),
        ("https://127.0.0.1", TargetError::WrongHost),
        ("https://", TargetError::WrongHost),
    ] {
        assert_eq!(Target::parse(url).unwrap_err(), why, "{url}");
    }
    assert!(HttpTransport::with_base("http://api.typesafe.ai", opts()).is_err());
    assert!(HttpTransport::with_base("https://evil.example", opts()).is_err());
}

#[test]
fn the_agent_is_configured_as_strictly_as_documented() {
    let c = agent_config(opts(), true);
    assert!(c.https_only());
    assert_eq!(c.max_redirects(), 0);
    assert!(
        c.proxy().is_none(),
        "no proxy, whatever the environment says"
    );
    assert!(
        !c.http_status_as_error(),
        "a status is read, not turned into a bare error"
    );
    assert!(!c.tls_config().disable_verification());
    assert_eq!(c.timeouts().global, Some(opts().total_timeout));
    assert_eq!(c.timeouts().connect, Some(opts().connect_timeout));
    // the shipped transport is https only; only the test constructor relaxes it
    assert!(
        HttpTransport::with_base("https://api.typesafe.ai", opts())
            .unwrap()
            .https_only()
    );
    assert!(!agent_config(opts(), false).https_only());
}

#[test]
fn defaults_are_a_hard_connect_and_total_timeout_and_a_bounded_reply() {
    let d = HttpOptions::default();
    assert!(d.connect_timeout > Duration::ZERO && d.connect_timeout <= Duration::from_secs(30));
    assert!(d.total_timeout >= d.connect_timeout && d.total_timeout <= Duration::from_secs(120));
    assert!(d.max_reply_bytes >= 64 * 1024 && d.max_reply_bytes <= 32 * 1024 * 1024);
}

#[test]
fn http_is_refused_before_any_connection_is_made() {
    let s = Server::start(|_| http("200 OK", &[], "{}"));
    let t = HttpTransport::loopback_for_tests(s.port, true, opts());
    assert!(matches!(
        t.send(&key(), &JevRequest::models()),
        Err(JevTransportError::Network)
    ));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        s.connections(),
        0,
        "the key must not cross a plain connection"
    );
}
