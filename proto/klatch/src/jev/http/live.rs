//! The live checks against the real API (#911, #918). Every test here is `#[ignore]`d: it needs `TYPESAFE_API_KEY`,
//! costs a few requests, and never runs in CI or in the gate. Run it by hand:
//!
//! ```text
//! doppler run --project governingdocs --config dev_alex -- cargo test -p klatch --features jev-http -- --ignored jev_live
//! ```
//!
//! (add `--nocapture` to see the lines as they happen; the same lines are always written to the report file whose
//! path is the last line printed and is `$TMPDIR/klatch-jev-live.txt`.)
//!
//! What it records for each call is the HTTP status, the `error_type` (only the three known values) and whether the
//! reply parsed. Never a body, a header value other than the shape of the retry hints, or any message text: the text
//! sent is made up (`SYNTHETIC`), and the picture is drawn by [`synthetic_png`], a red square on white.

use super::*;
use crate::jev::{
    Entry, Gate, Image, JevClient, JevConfig, JevError, Key, Question, build_request,
    judge_question,
};
use std::io::Write;
use std::sync::Mutex;

const MODEL: &str = "jev-1.14.0";
const SYNTHETIC: &str = "hello, can we schedule the weekly sync for tuesday?";

static REPORT: Mutex<()> = Mutex::new(());

fn report_path() -> std::path::PathBuf {
    std::env::temp_dir().join("klatch-jev-live.txt")
}

fn record(line: &str) {
    let _g = REPORT.lock().unwrap_or_else(|e| e.into_inner());
    eprintln!("jev_live {line}");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(report_path())
    {
        let _ = writeln!(f, "{line}");
    }
}

/// `status` and `error_type` of a result, and nothing else.
fn outcome<T>(r: &Result<T, JevError>) -> String {
    match r {
        Ok(_) => "status=2xx ok".into(),
        Err(e) => format!(
            "status={} error_type={} ({})",
            e.status().map_or("none".to_string(), |s| s.to_string()),
            e.error_type().map_or("none", |t| t.name()),
            match e {
                JevError::Malformed(_) => "reply refused by the parser",
                JevError::Unreachable => "no answer",
                JevError::NoKey => "no key",
                _ => "http",
            }
        ),
    }
}

fn gate() -> Gate {
    Gate::new(Default::default(), Some("internal"))
}

fn client(model: &str) -> JevClient<HttpTransport> {
    JevClient::new(HttpTransport::new(), JevConfig::new(model, 0.8).unwrap()).with_gate(gate())
}

fn text() -> Entry {
    Entry::text("public", SYNTHETIC)
}

// ---- a synthetic picture, with no dependency ----

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    let mut c = kind.to_vec();
    c.extend(data);
    out.extend(crc32(&c).to_be_bytes());
}

/// A valid RGB PNG of `side` x `side`: white with a red square in the middle third. The pixel data is zlib "stored"
/// blocks (no compression), which any decoder reads.
fn synthetic_png(side: u32) -> Vec<u8> {
    let (lo, hi) = (side / 3, 2 * side / 3);
    let mut raw = Vec::with_capacity((side * (side * 3 + 1)) as usize);
    for y in 0..side {
        raw.push(0); // filter: none
        for x in 0..side {
            let red = (lo..hi).contains(&x) && (lo..hi).contains(&y);
            raw.extend(if red {
                [220u8, 20, 20]
            } else {
                [255, 255, 255]
            });
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(b) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend((b.len() as u16).to_le_bytes());
        z.extend((!(b.len() as u16)).to_le_bytes());
        z.extend(b);
    }
    z.extend(adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = vec![];
    ihdr.extend(side.to_be_bytes());
    ihdr.extend(side.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn picture() -> Image {
    Image::from_bytes(synthetic_png(320))
        .expect("the synthetic picture passes the local image checks")
}

#[test]
fn the_synthetic_png_helpers_are_correct() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    let img = picture();
    assert_eq!((img.width(), img.height()), (320, 320));
    assert_eq!(img.content_type(), "image/png");
    // a second size also passes the local checks
    assert!(Image::from_bytes(synthetic_png(400)).is_ok());
}

// ---- the live run ----

#[test]
#[ignore = "needs TYPESAFE_API_KEY and calls the real API; run by hand (see the module text)"]
fn jev_live_all() {
    assert!(
        ApiKey::from_env().is_some(),
        "set TYPESAFE_API_KEY (doppler run --project governingdocs --config dev_alex -- ...)"
    );
    let _ = std::fs::remove_file(report_path());
    record(&format!("start model={MODEL} host={HOST}"));
    let c = client(MODEL);

    // the model list, and the key check through it
    let models = c.list_models();
    record(&format!("models: {}", outcome(&models)));
    if let Ok(m) = &models {
        record(&format!(
            "models: jev-latest listed={} jev-preview listed={}",
            m.contains("jev-latest"),
            m.contains("jev-preview")
        ));
    }
    assert!(
        models.is_ok(),
        "the key and the transport must work before anything else is worth recording"
    );
    record(&format!(
        "pinned model check: {:?}",
        c.check_model()
            .map(|s| format!("{s:?}"))
            .map_err(|e| outcome::<()>(&Err(e)))
    ));

    // the made-up text checks of #911: choice (the judge's question), noul
    let judge = judge_question();
    let r = c.ask(&text(), std::slice::from_ref(&judge));
    record(&format!("choice (judge question): {}", outcome(&r)));
    if let Ok(a) = &r
        && let Some(ch) = a.choice(crate::jev::JUDGE_QUESTION_ID)
    {
        record(&format!(
            "choice: confidence above threshold={}",
            ch.chosen_probability() >= 0.8
        ));
    }
    let noul = Question::noul("billing", "Is the message in the state about billing?");
    record(&format!("noul: {}", outcome(&c.ask(&text(), &[noul]))));

    // score
    let score = Question::score(
        "urgency",
        "How urgent is the message in the state?",
        &[
            "Can wait",
            "Needs attention this week",
            "Needs attention today",
        ],
    );
    let r = c.ask(&text(), &[score]);
    record(&format!("score: {}", outcome(&r)));
    if let Ok(a) = &r
        && let Some(s) = a.score("urgency")
    {
        record(&format!(
            "score: legend entries={} score in range={}",
            s.legend.len(),
            s.score >= 0.0 && s.score <= (s.legend.len().max(1) - 1) as f64
        ));
    }

    // images: in the state, in the instructions, in the criteria (multipart each time), and a box
    let in_state = Entry::object(vec![(
        Key::Fixed("picture"),
        Entry::image("public", picture()),
    )]);
    let r = c.ask(
        &in_state,
        &[Question::noul(
            "square",
            "Is there a red square in `picture`?",
        )],
    );
    record(&format!("image in state (multipart): {}", outcome(&r)));
    if let Ok(a) = &r {
        record(&format!(
            "image in state: usage reported={}",
            a.usage().is_some()
        ));
    }
    let instr = Entry::sentence(vec![
        Entry::fixed("Is there a red square in this picture: "),
        Entry::image("public", picture()),
        Entry::fixed("?"),
    ]);
    let r = c.ask(&text(), &[Question::noul_with("square", Some(instr), None)]);
    record(&format!(
        "image in instructions (multipart): {}",
        outcome(&r)
    ));
    let q = Question::choice_with(
        "kind",
        Some(Entry::fixed(
            "Which description matches the message in the state?",
        )),
        vec![
            (
                Key::Fixed("plain"),
                Entry::fixed("Plain words with no picture."),
            ),
            (
                Key::Fixed("picture"),
                Entry::sentence(vec![
                    Entry::fixed("It looks like this: "),
                    Entry::image("public", picture()),
                ]),
            ),
        ],
    );
    record(&format!(
        "image in criteria (multipart): {}",
        outcome(&c.ask(&text(), &[q]))
    ));
    let r = c.ask(
        &in_state,
        &[Question::locate(
            "square",
            "Where is the red square in `picture`?",
        )],
    );
    record(&format!(
        "box (needs the images header, sent): {}",
        outcome(&r)
    ));
    if let Ok(a) = &r
        && let Some(b) = a.boxed("square")
    {
        record(&format!(
            "box: confidence present={}",
            b.confidence.is_some()
        ));
    }
    // the same image as base64 JSON, to compare what the API says about the two forms
    let prepared = build_request(
        MODEL,
        &in_state,
        &[Question::noul(
            "square",
            "Is there a red square in `picture`?",
        )],
        &gate(),
        &Default::default(),
    )
    .unwrap();
    let base64_request = JevRequest {
        method: Method::Post,
        path: "/v1/systemone",
        images: true,
        body: Some(prepared.body().to_string()),
        files: vec![],
    };
    let key = ApiKey::from_env().unwrap();
    record(&format!(
        "image as base64 JSON: {}",
        transport_outcome(HttpTransport::new().send(&key, &base64_request))
    ));

    // a bad key, an unknown model
    let bad = JevClient::new(HttpTransport::new(), JevConfig::new(MODEL, 0.8).unwrap())
        .with_key_source(Box::new(|| Some("not-a-real-key".to_string())));
    let r = bad.list_models();
    record(&format!("bad key (models): {}", outcome(&r)));
    assert!(
        matches!(
            r,
            Err(JevError::Unauthorized { .. } | JevError::Forbidden { .. })
        ),
        "a bad key is a 401 or 403"
    );
    let r = bad.ask(
        &text(),
        &[Question::noul(
            "billing",
            "Is the message in the state about billing?",
        )],
    );
    record(&format!("bad key (systemone): {}", outcome(&r)));
    let unknown = client("jev-9.9.9");
    record(&format!(
        "unknown model: {}",
        outcome(&unknown.ask(
            &text(),
            &[Question::noul(
                "billing",
                "Is the message in the state about billing?"
            )]
        ))
    ));

    // a small burst, no retries, to look for a 429 and what it says
    let burst_request = JevRequest::system_one(
        &build_request(
            MODEL,
            &text(),
            &[Question::noul(
                "billing",
                "Is the message in the state about billing?",
            )],
            &gate(),
            &Default::default(),
        )
        .unwrap(),
    );
    let transport = HttpTransport::new();
    let counts: Mutex<std::collections::BTreeMap<String, usize>> = Mutex::default();
    let hints: Mutex<Vec<String>> = Mutex::default();
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for _ in 0..15 {
                    let r = transport.send(&key, &burst_request);
                    let name = match &r {
                        Ok(_) => "200".to_string(),
                        Err(JevTransportError::Network) => "network".to_string(),
                        Err(JevTransportError::Status {
                            status, headers, ..
                        }) => {
                            if matches!(status, 429 | 529) {
                                hints.lock().unwrap().push(format!(
                                    "{status}: retry-after {}, retry-after-ms {}",
                                    shape(headers.retry_after.as_deref()),
                                    shape(headers.retry_after_ms.as_deref())
                                ));
                            }
                            status.to_string()
                        }
                    };
                    *counts.lock().unwrap().entry(name).or_default() += 1;
                }
            });
        }
    });
    record(&format!(
        "burst of 60 over 4 threads, no retries: {:?}",
        counts.lock().unwrap()
    ));
    let hints = hints.lock().unwrap();
    record(&format!(
        "burst: 429 or 529 seen={} {:?}",
        !hints.is_empty(),
        hints.first()
    ));
    record(&format!("done; report at {}", report_path().display()));
}

/// Only the shape of a retry hint: absent, a number, or something else (such as an HTTP date).
fn shape(v: Option<&str>) -> &'static str {
    match v {
        None => "absent",
        Some(t) if t.trim().parse::<f64>().is_ok() => "a number",
        Some(_) => "not a number",
    }
}

fn transport_outcome(r: Result<String, JevTransportError>) -> String {
    match r {
        Ok(_) => "status=2xx ok".into(),
        Err(JevTransportError::Network) => "no answer".into(),
        Err(JevTransportError::Status { status, body, .. }) => {
            let e = crate::jev::http_error(status, &body);
            outcome::<()>(&Err(e))
        }
    }
}
