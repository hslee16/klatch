use rusqlite::{params, Connection};
use std::time::Instant;
use chacha20poly1305::{aead::{Aead, KeyInit, OsRng, AeadCore}, ChaCha20Poly1305};

fn schema(c: &Connection) {
    c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS messages(room TEXT, seq INTEGER, sender TEXT, ts INTEGER, body BLOB, PRIMARY KEY(room,seq));").unwrap();
}
fn pct(v: &mut Vec<f64>, p: f64) -> f64 { v.sort_by(|a,b| a.partial_cmp(b).unwrap()); v[((v.len() as f64 - 1.0) * p) as usize] }

fn bench(name: &str, c: &Connection, seal: &dyn Fn(&str) -> Vec<u8>, open: &dyn Fn(&[u8]) -> String) {
    let n = 5000;
    let t = Instant::now();
    let tx = c.unchecked_transaction().unwrap();
    for i in 0..n { c.execute("INSERT INTO messages VALUES(?1,?2,?3,?4,?5)", params!["lobby", i, "alice", 1000+i, seal(&format!("message number {i} with some plausible body text of moderate length"))]).unwrap(); }
    tx.commit().unwrap();
    let w = t.elapsed().as_secs_f64();
    let mut lat = Vec::new();
    for _ in 0..300 {
        let t = Instant::now();
        let mut st = c.prepare("SELECT body FROM messages WHERE room='lobby' AND seq>?1 ORDER BY seq LIMIT 20").unwrap();
        let rows: Vec<Vec<u8>> = st.query_map(params![4900], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
        let _: Vec<String> = rows.iter().map(|b| open(b)).collect();
        lat.push(t.elapsed().as_secs_f64()*1000.0);
    }
    let mut single = Vec::new();
    for i in 0..300 { let t = Instant::now(); c.execute("INSERT INTO messages VALUES('lobby',?1,'bob',1,?2)", params![10000+i, seal("one more message")]).unwrap(); single.push(t.elapsed().as_secs_f64()*1000.0); }
    println!("{name}: bulk insert {n} rows {:.0} ms ({:.0}/s); read20 p50 {:.3} ms p99 {:.3} ms; single insert p50 {:.3} ms p99 {:.3} ms", w*1000.0, n as f64/w, pct(&mut lat.clone(),0.5), pct(&mut lat,0.99), pct(&mut single.clone(),0.5), pct(&mut single,0.99));
}

fn main() {
    let mode = std::env::args().nth(1).unwrap();
    let path = std::env::args().nth(2).unwrap();
    let _ = std::fs::remove_file(&path); let _ = std::fs::remove_file(format!("{path}-wal")); let _ = std::fs::remove_file(format!("{path}-shm"));
    let c = Connection::open(&path).unwrap();
    match mode.as_str() {
        "plain" => { schema(&c); bench("plain", &c, &|s| s.as_bytes().to_vec(), &|b| String::from_utf8_lossy(b).into_owned()); }
        "sqlcipher" => {
            #[cfg(feature="sqlcipher")] {
                c.pragma_update(None, "key", "correct horse battery staple test key").unwrap();
                let v: String = c.query_row("PRAGMA cipher_version", [], |r| r.get(0)).unwrap(); println!("cipher_version {v}");
                schema(&c); bench("sqlcipher", &c, &|s| s.as_bytes().to_vec(), &|b| String::from_utf8_lossy(b).into_owned());
            }
        }
        "envelope" => {
            let key = ChaCha20Poly1305::generate_key(&mut OsRng); let cipher = ChaCha20Poly1305::new(&key);
            schema(&c);
            let seal = |s: &str| { let n = ChaCha20Poly1305::generate_nonce(&mut OsRng); let mut out = n.to_vec(); out.extend(cipher.encrypt(&n, s.as_bytes()).unwrap()); out };
            let open = |b: &[u8]| String::from_utf8_lossy(&cipher.decrypt(b[..12].into(), &b[12..]).unwrap()).into_owned();
            bench("envelope(field)", &c, &seal, &open);
        }
        "open" => {
            drop(c);
            #[cfg(feature="sqlcipher")] {
                let hexkey = "x'2DD29CA851E7B56E4697B0E1F08507293D761A05CE4D1B628663F411A8086D99'";
                { let c = Connection::open(&path).unwrap(); c.pragma_update(None,"key",hexkey).unwrap(); schema(&c); c.execute("INSERT INTO messages VALUES('lobby',1,'a',1,x'00')",[]).unwrap(); }
                for (label, key) in [("raw 256-bit key", hexkey.to_string())] {
                    let mut v = Vec::new();
                    for _ in 0..200 { let t = Instant::now(); let c = Connection::open(&path).unwrap(); c.pragma_update(None,"key",&key).unwrap(); let _: i64 = c.query_row("SELECT count(*) FROM messages", [], |r| r.get(0)).unwrap(); v.push(t.elapsed().as_secs_f64()*1000.0); }
                    println!("open+key+first query, {label}: p50 {:.3} ms p99 {:.3} ms", pct(&mut v.clone(),0.5), pct(&mut v,0.99));
                }
                // passphrase with the default KDF
                let _ = std::fs::remove_file(&path); let _ = std::fs::remove_file(format!("{path}-wal")); let _ = std::fs::remove_file(format!("{path}-shm"));
                { let c = Connection::open(&path).unwrap(); c.pragma_update(None,"key","a passphrase").unwrap(); schema(&c); c.execute("INSERT INTO messages VALUES('lobby',1,'a',1,x'00')",[]).unwrap(); }
                let mut v = Vec::new();
                for _ in 0..30 { let t = Instant::now(); let c = Connection::open(&path).unwrap(); c.pragma_update(None,"key","a passphrase").unwrap(); let _: i64 = c.query_row("SELECT count(*) FROM messages", [], |r| r.get(0)).unwrap(); v.push(t.elapsed().as_secs_f64()*1000.0); }
                println!("open+key+first query, passphrase (default KDF): p50 {:.3} ms p99 {:.3} ms", pct(&mut v.clone(),0.5), pct(&mut v,0.99));
            }
            #[cfg(feature="plain")] {
                { let c = Connection::open(&path).unwrap(); schema(&c); c.execute("INSERT INTO messages VALUES('lobby',1,'a',1,x'00')",[]).unwrap(); }
                let mut v = Vec::new();
                for _ in 0..200 { let t = Instant::now(); let c = Connection::open(&path).unwrap(); let _: i64 = c.query_row("SELECT count(*) FROM messages", [], |r| r.get(0)).unwrap(); v.push(t.elapsed().as_secs_f64()*1000.0); }
                println!("open+first query, plain: p50 {:.3} ms p99 {:.3} ms", pct(&mut v.clone(),0.5), pct(&mut v,0.99));
            }
            return;
        }
        _ => {}
    }
    drop(c);
    for f in [path.clone(), format!("{path}-wal")] { if let Ok(b) = std::fs::read(&f) { let hay = String::from_utf8_lossy(&b); println!("  {f}: {} bytes, plaintext 'plausible body' present: {}, 'lobby' present: {}, 'alice' present: {}", b.len(), hay.contains("plausible body"), hay.contains("lobby"), hay.contains("alice")); } }
}
