//! Recognising text that carries a credential, so the gateway can refuse to store it (R20, ticket #31).
//!
//! Only well known shapes are matched, each with a prefix and a minimum length, so ordinary prose, hashes and
//! identifiers pass. A match names the kind of credential and never the text.

/// What kind of credential `text` appears to carry, if any.
pub fn credential_kind(text: &str) -> Option<&'static str> {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY-----") {
        return Some("a private key");
    }
    SHAPES
        .iter()
        .find(|s| s.matches(text))
        .map(|s| s.kind)
        .or_else(|| bearer_token(text).then_some("a bearer token"))
        .or_else(|| json_web_token(text).then_some("a JSON web token"))
}

/// A credential that starts with a fixed prefix and then runs for at least `min` characters of a class.
struct Shape {
    kind: &'static str,
    prefix: &'static str,
    min: usize,
    class: fn(char) -> bool,
}

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn upper_alnum(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit()
}

fn token(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

const SHAPES: [Shape; 13] = [
    Shape {
        kind: "an AWS access key",
        prefix: "AKIA",
        min: 16,
        class: upper_alnum,
    },
    Shape {
        kind: "an AWS access key",
        prefix: "ASIA",
        min: 16,
        class: upper_alnum,
    },
    Shape {
        kind: "a GitHub token",
        prefix: "ghp_",
        min: 30,
        class: alnum,
    },
    Shape {
        kind: "a GitHub token",
        prefix: "gho_",
        min: 30,
        class: alnum,
    },
    Shape {
        kind: "a GitHub token",
        prefix: "ghu_",
        min: 30,
        class: alnum,
    },
    Shape {
        kind: "a GitHub token",
        prefix: "ghs_",
        min: 30,
        class: alnum,
    },
    Shape {
        kind: "a GitHub token",
        prefix: "github_pat_",
        min: 22,
        class: token,
    },
    Shape {
        kind: "a Slack token",
        prefix: "xoxb-",
        min: 10,
        class: token,
    },
    Shape {
        kind: "a Slack token",
        prefix: "xoxp-",
        min: 10,
        class: token,
    },
    Shape {
        kind: "an API key",
        prefix: "sk-",
        min: 24,
        class: token,
    },
    Shape {
        kind: "a Stripe key",
        prefix: "sk_live_",
        min: 16,
        class: alnum,
    },
    Shape {
        kind: "a Stripe key",
        prefix: "rk_live_",
        min: 16,
        class: alnum,
    },
    Shape {
        kind: "a Google API key",
        prefix: "AIza",
        min: 35,
        class: token,
    },
];

impl Shape {
    fn matches(&self, text: &str) -> bool {
        text.match_indices(self.prefix).any(|(i, _)| {
            let standalone = text[..i].chars().next_back().is_none_or(|c| !alnum(c));
            let run = text[i + self.prefix.len()..]
                .chars()
                .take_while(|c| (self.class)(*c))
                .count();
            standalone && run >= self.min
        })
    }
}

/// `Authorization: Bearer <token>` or `Bearer <token>` with a long token.
fn bearer_token(text: &str) -> bool {
    text.match_indices("Bearer ").any(|(i, _)| {
        let run = text[i + 7..]
            .chars()
            .take_while(|c| token(*c) || matches!(c, '.' | '~' | '+' | '/' | '='))
            .count();
        run >= 20
    })
}

/// Three dot separated base64url parts, the first starting with the encoded `{"`: a JSON web token.
fn json_web_token(text: &str) -> bool {
    text.split(|c: char| !(token(c) || c == '.')).any(|word| {
        let parts: Vec<&str> = word.split('.').collect();
        parts.len() == 3 && word.starts_with("eyJ") && parts.iter().all(|p| p.len() >= 8)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_credential_shapes_are_recognised_by_kind() {
        let cases = [
            ("-----BEGIN OPENSSH PRIVATE KEY-----\nabc", "a private key"),
            ("key AKIAIOSFODNN7EXAMPLE here", "an AWS access key"),
            (
                "token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
                "a GitHub token",
            ),
            ("github_pat_11ABCDEFG0abcdefghijklmnop", "a GitHub token"),
            ("xoxb-1234567890-abcdefghij", "a Slack token"),
            ("use sk-abcdefghijklmnopqrstuvwxyz123456", "an API key"),
            ("sk-ant-api03-abcdefghijklmnopqrstuvwx", "an API key"),
            ("sk_live_abcdefghijklmnop1234", "a Stripe key"),
            (
                "AIzaSyA-abcdefghijklmnopqrstuvwxyz012345",
                "a Google API key",
            ),
            (
                "Authorization: Bearer abcdefghijklmnopqrstuvwxyz",
                "a bearer token",
            ),
            (
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnop",
                "a JSON web token",
            ),
        ];
        for (text, kind) in cases {
            assert_eq!(credential_kind(text), Some(kind), "{text:?}");
        }
    }

    #[test]
    fn ordinary_text_hashes_and_identifiers_are_not_credentials() {
        let cases = [
            "seeking: a slugify helper",
            "see commit 3f9c1d2a8b7e6f5d4c3b2a1908f7e6d5c4b3a291",
            "task-abcdefghijklmnopqrstuvwxyz12345",
            "the sk- prefix is short here: sk-abc",
            "AKIA is the prefix of an AWS key",
            "a Bearer of bad news",
            "eyJ is how base64 json starts",
            "id 123e4567-e89b-12d3-a456-426614174000",
            "",
        ];
        for text in cases {
            assert_eq!(credential_kind(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_match_names_the_kind_and_never_the_text() {
        let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let kind = credential_kind(secret).unwrap();
        assert!(!kind.contains("ghp_") && !kind.contains("abcdef"));
    }
}
