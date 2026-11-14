//! The session loop under hostile input (#165): garbage lines never stop it, never panic it, and a valid request after
//! the garbage is still answered.

use klatch::domain::Principal;
use klatch::gateway::{AllowAll, FixedClock, Gateway};
use klatch::protocol::{ServerFrame, decode_server_line};
use klatch::session::serve;
use klatch::store::SqliteStore;
use std::io::Cursor;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn garbage(rng: &mut Rng) -> Vec<u8> {
    let n = (rng.next() % 300) as usize;
    let mut v: Vec<u8> = (0..n).map(|_| (rng.next() & 0xFF) as u8).collect();
    v.retain(|b| *b != b'\n');
    v
}

#[test]
fn garbage_between_valid_requests_never_stops_the_session() {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1_000),
        AllowAll,
    );
    let who = Principal::unattributed("alice");
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for round in 0..40 {
        let mut input = Vec::new();
        for _ in 0..(round % 9) {
            input.extend(garbage(&mut rng));
            input.push(b'\n');
        }
        // JSON-shaped nonsense, a huge line, and a request with the wrong version
        input.extend(br#"{"v":1,"type":"request","id":"a","method":"no-such-method","params":{}}"#);
        input.push(b'\n');
        input
            .extend(br#"{"v":99,"type":"request","id":"b","method":"join","params":{"room":"r"}}"#);
        input.push(b'\n');
        input.extend(std::iter::repeat_n(b'z', 1_500_000));
        input.push(b'\n');
        input.extend(
            br#"{"v":1,"type":"request","id":"last","method":"join","params":{"room":"r"}}"#,
        );
        input.push(b'\n');
        let mut out = Vec::new();
        serve(&g, &who, Cursor::new(input), &mut out).unwrap();
        let frames: Vec<ServerFrame> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| decode_server_line(l).unwrap_or_else(|e| panic!("bad output {l:?}: {e:?}")))
            .collect();
        let answered = frames
            .iter()
            .any(|f| matches!(f, ServerFrame::Response { id, .. } if id == "last"));
        assert!(
            answered,
            "round {round}: the request after the garbage was not answered: {frames:#?}"
        );
    }
}

#[test]
fn input_that_is_only_garbage_ends_with_the_end_of_input() {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1_000),
        AllowAll,
    );
    let who = Principal::unattributed("alice");
    let mut rng = Rng(42);
    let mut input = Vec::new();
    for _ in 0..200 {
        input.extend(garbage(&mut rng));
        input.push(b'\n');
    }
    let mut out = Vec::new();
    serve(&g, &who, Cursor::new(input), &mut out).unwrap();
}
