use super::*;

mod multipart {
    use super::*;
    use crate::jev::transport::{FilePart, MULTIPART_OVERHEAD, multipart_body};

    fn part(name: &str, bytes: &[u8]) -> FilePart {
        FilePart::new(name.into(), "image/png", bytes.to_vec())
    }

    fn find(h: &[u8], n: &[u8]) -> Option<usize> {
        h.windows(n.len()).position(|w| w == n)
    }

    #[test]
    fn the_first_part_is_request_json_then_one_file_part_each_with_its_content_type() {
        let files = [part("image-0000", b"AAAA"), part("image-0001", b"BBBB")];
        let (ct, body) = multipart_body(r#"{"a":1}"#, &files);
        let boundary = ct.strip_prefix("multipart/form-data; boundary=").unwrap();
        let text = String::from_utf8_lossy(&body).to_string();
        let req = text.find("name=\"request\"").unwrap();
        let i0 = text.find("name=\"image-0000\"").unwrap();
        let i1 = text.find("name=\"image-0001\"").unwrap();
        assert!(req < i0 && i0 < i1, "request first, files in order");
        assert!(text.contains("Content-Type: application/json\r\n\r\n{\"a\":1}\r\n"));
        assert!(text.contains("Content-Type: image/png\r\n\r\nAAAA\r\n"));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
        assert_eq!(text.matches(&format!("--{boundary}\r\n")).count(), 3);
        assert!(body.len() <= 7 + 8 + MULTIPART_OVERHEAD * 3);
    }

    #[test]
    fn the_boundary_never_occurs_in_the_data() {
        // a file that contains the first boundary candidate forces another one
        let (ct0, _) = multipart_body("{}", &[]);
        let first = ct0.strip_prefix("multipart/form-data; boundary=").unwrap();
        let nasty = part("image-0000", format!("xx--{first}yy").as_bytes());
        let (ct, body) = multipart_body("{}", &[nasty]);
        let b = ct.strip_prefix("multipart/form-data; boundary=").unwrap();
        assert_ne!(b, first);
        let delim = format!("--{b}");
        // exactly: 2 part openings and the closing line
        assert_eq!(
            body.windows(delim.len())
                .filter(|w| *w == delim.as_bytes())
                .count(),
            3
        );
        assert!(find(&body, format!("xx--{first}yy").as_bytes()).is_some());
    }

    #[test]
    fn json_in_the_request_part_cannot_collide_either() {
        let (ct0, _) = multipart_body("{}", &[]);
        let first = ct0.strip_prefix("multipart/form-data; boundary=").unwrap();
        let (ct, _) = multipart_body(&format!("{{\"x\":\"--{first}\"}}"), &[]);
        assert_ne!(
            ct.strip_prefix("multipart/form-data; boundary=").unwrap(),
            first
        );
    }

    #[test]
    fn a_request_with_files_is_multipart_and_has_no_json_content_type_header() {
        let st = Entry::object(vec![(
            Key::Fixed("d"),
            Entry::image("public", super::png()),
        )]);
        let q = Question::noul("n", "is it there?");
        let p = build_request(
            "jev-1.14.0",
            &st,
            &[q],
            &Gate::new(Default::default(), Some("internal")),
            &Limits::default(),
        )
        .unwrap();
        let r = JevRequest::system_one(&p);
        assert!(r.images);
        assert_eq!(r.files.len(), 1);
        assert!(r.body.as_ref().unwrap().contains("\"part\""));
        assert!(!r.body.as_ref().unwrap().contains("data_base64"));
        assert!(r.headers().iter().all(|(k, _)| *k != "Content-Type"));
        assert!(r.multipart().is_some());
        let plain = JevRequest::models();
        assert!(plain.multipart().is_none());
    }

    #[test]
    fn debug_never_shows_file_bytes() {
        let f = part("image-0000", b"SECRETBYTES");
        assert!(!format!("{f:?}").contains("SECRETBYTES"));
    }
}
