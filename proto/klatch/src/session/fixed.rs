//! A session whose member is fixed by its transport (#107, an ssh forced command) refuses a request that tries to say
//! who is calling. The protocol already gives such a field no effect (PROTOCOL.md section 5); on a fixed session it
//! is refused, so an attempt is visible to the peer and to anyone reading a script of the session.

use serde_json::Value;

/// Request parameters that claim an identity, whatever the method. Compared without regard to case.
const CLAIMS: [&str; 10] = [
    "sender",
    "from",
    "as",
    "as_member",
    "caller",
    "identity",
    "principal",
    "attribution",
    "asserted_member",
    "key_label",
];

/// Methods whose `member` parameter names another member as the target of the act (the one invited, the one granted
/// a clearance, the one a role act is about), not the caller.
const MEMBER_IS_TARGET: [&str; 3] = ["invite", "grant_room_clearance", "role_act"];

/// The parameter of the request that claims who is calling, if there is one.
pub(super) fn identity_claim(method: &str, params: &Value) -> Option<String> {
    let object = params.as_object()?;
    object.keys().find_map(|key| {
        let lower = key.to_ascii_lowercase();
        let claims = CLAIMS.contains(&lower.as_str())
            || (lower == "member" && !MEMBER_IS_TARGET.contains(&method));
        claims.then(|| key.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_plain_request_claims_nothing() {
        assert_eq!(
            identity_claim("post", &json!({"room":"r","body":"x"})),
            None
        );
        assert_eq!(identity_claim("who", &Value::Null), None);
    }

    #[test]
    fn identity_fields_are_found_in_any_case() {
        for key in ["sender", "From", "AS", "member", "Principal"] {
            let p = json!({ key: "bob" });
            assert_eq!(identity_claim("post", &p).as_deref(), Some(key));
        }
    }

    #[test]
    fn member_names_a_target_for_the_methods_that_take_one() {
        let p = json!({"member":"bob"});
        for m in MEMBER_IS_TARGET {
            assert_eq!(identity_claim(m, &p), None, "{m}");
        }
        assert!(identity_claim("post", &p).is_some());
        // but a sender field is a claim there too
        assert!(identity_claim("invite", &json!({"sender":"bob"})).is_some());
    }
}
