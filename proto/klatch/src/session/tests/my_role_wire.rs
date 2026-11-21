//! The `my_role` wire method (#1110): a wire-only member reads its own role, the same call as `klatch role`.

use super::*;

fn call(
    g: &Gateway<SqliteStore>,
    who: &Principal,
    params: Value,
) -> (bool, Option<Value>, Option<(String, String)>) {
    response(&run(g, who, &[req("x", "my_role", params)]), "x")
}

#[test]
fn a_member_reads_its_own_role_and_null_without_one() {
    let g = gateway();
    g.join(&alice(), "lobby").unwrap();
    let (ok, result, err) = call(&g, &alice(), json!({}));
    assert!(ok, "{err:?}");
    assert_eq!(result.unwrap(), json!({"role": null}));
    let human = Principal::unattributed("admin");
    g.admin_set_integrator(&human, "alice", true).unwrap();
    let (_, result, _) = call(&g, &alice(), json!({}));
    assert_eq!(result.unwrap(), json!({"role": "integrator"}));
}

#[test]
fn another_members_role_is_not_readable() {
    let g = gateway();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g.admin_set_integrator(&Principal::unattributed("admin"), "alice", true)
        .unwrap();
    let (_, result, _) = call(&g, &bob(), json!({}));
    assert_eq!(result.unwrap(), json!({"role": null}));
    // A param naming another member is not read: the answer is still the caller's own.
    let (ok, result, err) = call(&g, &bob(), json!({"member": "alice"}));
    assert!(ok, "{err:?}");
    assert_eq!(result.unwrap(), json!({"role": null}));
}
