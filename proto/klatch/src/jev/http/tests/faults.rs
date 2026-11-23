use super::*;

#[test]
fn a_stalled_server_ends_at_the_total_timeout() {
    let s = Server::start(|_| Reply::Hang);
    let t = HttpTransport::loopback_for_tests(
        s.port,
        false,
        HttpOptions {
            total_timeout: Duration::from_millis(400),
            ..opts()
        },
    );
    let at = Instant::now();
    assert!(matches!(
        t.send(&key(), &JevRequest::models()),
        Err(JevTransportError::Network)
    ));
    assert!(at.elapsed() < Duration::from_secs(3), "{:?}", at.elapsed());
}

#[test]
fn a_closed_port_is_a_network_error_with_no_detail() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let t = HttpTransport::loopback_for_tests(port, false, opts());
    let e = t.send(&key(), &JevRequest::models()).unwrap_err();
    assert_eq!(e, JevTransportError::Network);
}

#[test]
fn an_oversized_reply_is_refused_by_length_and_when_chunked() {
    let o = HttpOptions {
        max_reply_bytes: 1024,
        ..opts()
    };
    let big = "x".repeat(5000);
    let s = Server::start({
        let big = big.clone();
        move |_| http("200 OK", &[], &big)
    });
    let t = HttpTransport::loopback_for_tests(s.port, false, o);
    assert_eq!(
        t.send(&key(), &JevRequest::models()).unwrap_err(),
        JevTransportError::Network
    );
    // chunked with no length: the bound still holds
    let chunked = {
        let mut b = String::new();
        for _ in 0..50 {
            b += &format!("{:x}\r\n{}\r\n", 100, "y".repeat(100));
        }
        b += "0\r\n\r\n";
        b
    };
    let s = Server::start(move |_| http("200 OK", &["Transfer-Encoding: chunked"], &chunked));
    let t = HttpTransport::loopback_for_tests(s.port, false, o);
    assert_eq!(
        t.send(&key(), &JevRequest::models()).unwrap_err(),
        JevTransportError::Network
    );
    // a reply at the bound is fine
    let ok = "z".repeat(1024);
    let s = Server::start(move |_| http("200 OK", &[], &ok));
    let t = HttpTransport::loopback_for_tests(s.port, false, o);
    assert_eq!(t.send(&key(), &JevRequest::models()).unwrap().len(), 1024);
}

#[test]
fn an_error_body_is_bounded_and_only_read_for_400_401_403() {
    let echo = format!(r#"{{"detail":[{{"input":{{"state":"{KEY} and the message"}}}}]}}"#);
    let s = Server::start(move |_| http("422 Unprocessable Entity", &[], &echo));
    let (status, body, _) = status_of(
        loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err(),
    );
    assert_eq!(status, 422);
    assert_eq!(
        body, "",
        "a 422 body echoes the request, so it is dropped here"
    );
    let s = Server::start(|_| http("500 Internal Server Error", &[], "internal trace"));
    assert_eq!(
        status_of(
            loopback(&s)
                .send(&key(), &JevRequest::models())
                .unwrap_err()
        )
        .1,
        ""
    );
    let s = Server::start(|_| http("401 Unauthorized", &[], fixtures::ERR_AUTH));
    let (st, body, _) = status_of(
        loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err(),
    );
    assert_eq!(st, 401);
    assert_eq!(body, fixtures::ERR_AUTH);
    // a 400 body over the bound is not kept at all
    let huge = format!(
        r#"{{"detail":{{"error_type":"api_usage_error","m":"{}"}}}}"#,
        "a".repeat(200_000)
    );
    let s = Server::start(move |_| http("400 Bad Request", &[], &huge));
    let (st, body, _) = status_of(
        loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err(),
    );
    assert_eq!(st, 400);
    assert!(body.len() <= opts().max_reply_bytes);
}

#[test]
fn the_retry_headers_come_through() {
    let s = Server::start(|_| {
        http(
            "429 Too Many Requests",
            &["Retry-After: 3", "retry-after-ms: 250"],
            "{}",
        )
    });
    let (st, _, h) = status_of(
        loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err(),
    );
    assert_eq!(st, 429);
    assert_eq!(h.retry_after.as_deref(), Some("3"));
    assert_eq!(h.retry_after_ms.as_deref(), Some("250"));
}

#[test]
fn a_content_encoded_reply_is_refused_because_none_was_asked_for() {
    let s = Server::start(|_| http("200 OK", &["Content-Encoding: gzip"], "not really gzip"));
    assert_eq!(
        loopback(&s)
            .send(&key(), &JevRequest::models())
            .unwrap_err(),
        JevTransportError::Network
    );
}
