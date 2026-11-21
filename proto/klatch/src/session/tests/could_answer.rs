//! The `could_answer` wire method (#671): the same read-only check as `klatch could-answer`, ids and rule words only.

use super::*;

fn call(
    g: &Gateway<SqliteStore>,
    who: &Principal,
    params: Value,
) -> (bool, Option<Value>, Option<(String, String)>) {
    response(&run(g, who, &[req("c", "could_answer", params)]), "c")
}

fn post_ask(g: &Gateway<SqliteStore>) -> i64 {
    let f = run(
        g,
        &alice(),
        &[req(
            "p",
            "post",
            json!({"room":"r","body":"seeking: SECRET-WORDING"}),
        )],
    );
    response(&f, "p").1.unwrap()["seq"].as_i64().unwrap()
}

#[test]
fn a_member_checking_itself_gets_the_answer_with_no_ask_text() {
    let g = gateway();
    let seq = post_ask(&g);
    let (ok, result, err) = call(&g, &bob(), json!({"room":"r","member":"bob","ask":seq}));
    assert!(ok, "{err:?}");
    let r = result.unwrap();
    assert_eq!(
        r,
        json!({"member":"bob","ask":seq,"could":true,"blockers":[]})
    );
    let (_, result, _) = call(&g, &alice(), json!({"room":"r","member":"alice","ask":seq}));
    let r = result.unwrap();
    assert_eq!(r["could"], json!(false));
    assert_eq!(r["blockers"][0]["rule"], json!("own-ask"));
    assert!(!r.to_string().contains("SECRET-WORDING"));
}

#[test]
fn a_missing_param_is_invalid() {
    let g = gateway();
    let (ok, _, err) = call(&g, &bob(), json!({"room":"r","member":"bob"}));
    assert!(!ok);
    assert!(err.is_some());
}

#[test]
fn a_room_above_the_callers_clearance_is_refused_when_checking_oneself() {
    let g = gateway();
    let seq = post_ask(&g);
    g.admin_set_room_label(&Principal::unattributed("admin"), "r", "internal")
        .unwrap();
    let (ok, _, err) = call(&g, &bob(), json!({"room":"r","member":"bob","ask":seq}));
    assert!(!ok);
    assert_eq!(err.unwrap().0, "refused");
}
