use super::*;

const SENTINEL: &str = "ECHOED-MESSAGE-TEXT";

fn body_with(error_type: &str) -> String {
    format!(r#"{{"detail": {{"error_type": "{error_type}", "message": "{SENTINEL}"}}}}"#)
}

#[test]
fn every_documented_status_has_its_own_typed_error() {
    let e = |s| http_error(s, "");
    assert_eq!(e(400), JevError::BadRequest { error_type: None });
    assert_eq!(e(401), JevError::Unauthorized { error_type: None });
    assert_eq!(e(403), JevError::Forbidden { error_type: None });
    assert_eq!(e(413), JevError::PayloadTooLarge);
    assert_eq!(e(415), JevError::UnsupportedMediaType);
    assert_eq!(e(422), JevError::Unprocessable);
    assert_eq!(e(429), JevError::RateLimited);
    assert_eq!(e(529), JevError::Overloaded);
    assert_eq!(e(500), JevError::Http { status: 500 });
    assert_eq!(e(404), JevError::Http { status: 404 });
    for s in [400, 401, 403, 413, 415, 422, 429, 529, 500, 404] {
        assert_eq!(e(s).status(), Some(s));
    }
    assert_eq!(JevError::NoKey.status(), None);
    assert_eq!(JevError::Unreachable.status(), None);
}

#[test]
fn the_error_type_is_read_for_400_401_and_403_only_and_only_the_known_ones() {
    assert_eq!(
        http_error(401, &body_with("authentication_error")),
        JevError::Unauthorized {
            error_type: Some(ErrorType::Authentication)
        }
    );
    assert_eq!(
        http_error(403, &body_with("authentication_error")),
        JevError::Forbidden {
            error_type: Some(ErrorType::Authentication)
        }
    );
    assert_eq!(
        http_error(400, &body_with("api_usage_error")),
        JevError::BadRequest {
            error_type: Some(ErrorType::ApiUsage)
        }
    );
    assert_eq!(
        http_error(400, &body_with("something_new")),
        JevError::BadRequest { error_type: None }
    );
    // the live reply to a 500,000 character state has no message at all
    assert_eq!(
        http_error(400, r#"{"detail":{"error_type":"max_tokens_exceeded"}}"#),
        JevError::ContextExceeded
    );
    // a body that is not the known shape says nothing
    for b in [
        "",
        "not json",
        "[]",
        r#"{"detail": "text"}"#,
        r#"{"detail": {"error_type": 5}}"#,
    ] {
        assert_eq!(
            http_error(400, b),
            JevError::BadRequest { error_type: None },
            "{b}"
        );
    }
    // every other status never reads its body, even when it holds a known error_type
    for s in [413, 415, 422, 429, 529, 500] {
        let e = http_error(s, &body_with("authentication_error"));
        assert_eq!(e.error_type(), None, "{s}");
    }
    // max_tokens_exceeded is a context error only as a 400
    assert_eq!(
        http_error(401, r#"{"detail":{"error_type":"max_tokens_exceeded"}}"#).error_type(),
        Some(ErrorType::MaxTokensExceeded)
    );
}

#[test]
fn no_error_ever_holds_any_part_of_a_body() {
    for s in [400, 401, 403, 413, 415, 422, 429, 529, 500] {
        let echo = format!(
            r#"{{"detail": [{{"input": "{SENTINEL}", "msg": "{SENTINEL}"}}], "x": "{SENTINEL}"}}"#
        );
        for b in [echo, body_with("api_usage_error")] {
            let e = http_error(s, &b);
            let shown = format!("{e} {e:?}");
            assert!(!shown.contains(SENTINEL), "{s}: {shown}");
        }
    }
    let t = JevTransportError::Status {
        status: 422,
        body: SENTINEL.into(),
        headers: RetryHeaders::default(),
    };
    assert!(!format!("{t:?}").contains(SENTINEL));
    assert!(format!("{t:?}").contains("422"));
}

#[test]
fn only_a_rate_limit_or_an_overload_is_worth_retrying() {
    for s in 100..600u16 {
        let retry = matches!(s, 429 | 529);
        assert_eq!(http_error(s, "").is_retryable(), retry, "{s}");
    }
    assert!(!JevError::ContextExceeded.is_retryable());
    assert!(!JevError::Unreachable.is_retryable());
}

#[test]
fn display_names_the_status_and_the_type_and_nothing_else() {
    assert_eq!(
        http_error(429, "").to_string(),
        "Jev answered HTTP 429 (rate limited)"
    );
    assert_eq!(
        http_error(529, "").to_string(),
        "Jev answered HTTP 529 (overloaded)"
    );
    assert_eq!(
        http_error(400, &body_with("api_usage_error")).to_string(),
        "Jev answered HTTP 400 (api_usage_error)"
    );
    assert!(
        http_error(400, r#"{"detail":{"error_type":"max_tokens_exceeded"}}"#)
            .to_string()
            .contains("max_tokens_exceeded")
    );
    assert!(http_error(422, "").to_string().contains("422"));
    assert_eq!(http_error(500, "").to_string(), "Jev answered HTTP 500");
}
