use super::*;

#[test]
fn a_get_sends_the_key_only_in_the_authorization_header() {
    let s = Server::start(|_| http("200 OK", &[], fixtures::LIVE_MODELS));
    let body = loopback(&s).send(&key(), &JevRequest::models()).unwrap();
    assert_eq!(body, fixtures::LIVE_MODELS);
    let head = s.head(0);
    assert!(head.starts_with("GET /v1/models HTTP/1.1"), "{head}");
    assert!(
        head.contains(&format!("authorization: Bearer {KEY}"))
            || head.contains(&format!("Authorization: Bearer {KEY}"))
    );
    assert_eq!(
        head.matches(KEY).count(),
        1,
        "the key is in no other header or the path"
    );
    assert!(!head.to_ascii_lowercase().contains("content-type"));
    assert!(!head.to_ascii_lowercase().contains("proxy"));
    assert!(
        !head.to_ascii_lowercase().contains("accept-encoding"),
        "no compression is negotiated"
    );
}

#[test]
fn a_json_post_carries_the_body_the_content_type_and_the_images_header_only_when_asked() {
    let s = Server::start(|_| http("200 OK", &[], "{}"));
    let r = JevRequest {
        method: Method::Post,
        path: "/v1/systemone",
        images: false,
        body: Some(r#"{"model":"m"}"#.into()),
        files: vec![],
    };
    loopback(&s).send(&key(), &r).unwrap();
    let head = s.head(0).to_ascii_lowercase();
    assert!(head.starts_with("post /v1/systemone"));
    assert!(head.contains("content-type: application/json"));
    assert!(!head.contains("typesafe-danger-zone-images"));
    assert!(String::from_utf8_lossy(&s.request(0)).ends_with(r#"{"model":"m"}"#));
    let r = JevRequest { images: true, ..r };
    loopback(&s).send(&key(), &r).unwrap();
    assert!(
        s.head(1)
            .to_ascii_lowercase()
            .contains("typesafe-danger-zone-images: true")
    );
}

#[test]
fn an_image_request_goes_as_multipart_with_the_file_as_it_is() {
    let s = Server::start(|_| http("200 OK", &[], "{}"));
    let bytes = vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3, 255];
    let r = JevRequest {
        method: Method::Post,
        path: "/v1/systemone",
        images: true,
        body: Some(r#"{"model":"m"}"#.into()),
        files: vec![FilePart::new(
            "image-0000".into(),
            "image/png",
            bytes.clone(),
        )],
    };
    loopback(&s).send(&key(), &r).unwrap();
    let head = s.head(0).to_ascii_lowercase();
    assert!(
        head.contains("content-type: multipart/form-data; boundary=klatch-jev-boundary-"),
        "{head}"
    );
    assert!(head.contains("typesafe-danger-zone-images: true"));
    let raw = s.request(0);
    assert!(
        raw.windows(bytes.len()).any(|w| w == bytes.as_slice()),
        "raw bytes, not base64"
    );
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("name=\"request\"") && text.contains("name=\"image-0000\""));
    assert!(!text.contains("data_base64"));
}

#[test]
fn a_redirect_is_refused_and_never_followed() {
    let target = Server::start(|_| http("200 OK", &[], "{}"));
    let url = format!("Location: http://127.0.0.1:{}/v1/models", target.port);
    let s = Server::start(move |_| http("302 Found", &[&url], ""));
    for code in [
        "301 Moved",
        "302 Found",
        "303 See Other",
        "307 Temporary",
        "308 Permanent",
    ] {
        let url = format!("Location: http://127.0.0.1:{}/v1/models", target.port);
        let c = code.to_string();
        let s2 = Server::start(move |_| http(&c, &[&url], ""));
        let e = loopback(&s2)
            .send(&key(), &JevRequest::models())
            .unwrap_err();
        assert_eq!(status_of(e).0, code[..3].parse::<u16>().unwrap());
        assert_eq!(s2.connections(), 1);
    }
    let _ = s;
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        target.connections(),
        0,
        "the redirect target was never contacted, so the key never went there"
    );
}
