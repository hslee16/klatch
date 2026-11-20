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
        .or_else(|| url_or_command_password(text).then_some("a password"))
        .or_else(|| labelled_secret(text).then_some("a labelled secret"))
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

const SHAPES: [Shape; 14] = [
    Shape {
        kind: "an npm token",
        prefix: "npm_",
        min: 36,
        class: alnum,
    },
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

/// A value that is plainly a placeholder or a reference rather than a secret.
fn placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.starts_with(['<', '{', '$', '*', '[', '(', '.', '>', '}'])
        || (value.starts_with('%') && (value.ends_with('%') || value.len() < 3))
        || value
            .chars()
            .all(|c| matches!(c, '*' | 'x' | 'X' | '.' | '#'))
        || matches!(
            lower.as_str(),
            "pass" | "password" | "secret" | "passwd" | "changeme" | "redacted"
        )
}

fn trim_value(word: &str) -> &str {
    word.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | ',' | ';' | ')' | '(' | '.'))
}

/// A password in a connection URL (`scheme://user:PASSWORD@host`), after `-u user:PASSWORD` or `--user` of a
/// curl command, in `mysql -pPASSWORD`, or in `--password=PASSWORD`.
fn url_or_command_password(text: &str) -> bool {
    let real = |p: &str| p.len() >= 3 && !placeholder(p);
    let in_url = text.match_indices("://").any(|(i, _)| {
        let rest = &text[i + 3..];
        let authority = rest
            .split(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
            .next()
            .unwrap_or("");
        authority
            .rsplit_once('@')
            .and_then(|(info, _)| info.split_once(':'))
            .is_some_and(|(_, pw)| real(pw))
    });
    if in_url {
        return true;
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let curl_user = text.contains("curl")
        && words.windows(2).any(|w| {
            matches!(w[0], "-u" | "--user")
                && w[1]
                    .split_once(':')
                    .is_some_and(|(_, pw)| real(trim_value(pw)))
        });
    let mysql_flag = text.contains("mysql")
        && words.iter().any(|w| {
            w.strip_prefix("-p")
                .is_some_and(|pw| !pw.starts_with('-') && pw.len() >= 4 && real(trim_value(pw)))
        });
    let password_flag = words.iter().any(|w| {
        w.strip_prefix("--password=")
            .is_some_and(|pw| real(trim_value(pw)))
    });
    curl_user || mysql_flag || password_flag
}

/// A value that reads as a secret after a label: long enough, not a placeholder, and with a digit or a symbol
/// (so a hyphenated name or a path such as `billing-db` or `secret/prod/db` is not one).
fn secret_looking(value: &str) -> bool {
    value.len() >= 8
        && !placeholder(value)
        && value
            .chars()
            .any(|c| c.is_ascii_digit() || "!#%^&*+=~@$".contains(c))
}

/// Words that label a secret in prose, configuration and logs.
const LABELS: [&str; 8] = [
    "pass",
    "password",
    "passphrase",
    "passwd",
    "passwort",
    "pw",
    "sessionid",
    "secret",
];

/// Labels after which only a long base64 value counts (a bare word like `token` is too common in prose).
const BLOB_LABELS: [&str; 3] = ["token", "apikey", "api_key"];

/// The label starts a word (an `_` or `.` before it is fine) and is not the front of a longer word or a compound
/// name (`secret-marker-7731`: a hyphen right after it is not a separator).
fn whole_label(lower: &str, at: usize, len: usize) -> bool {
    let before = lower[..at].chars().next_back();
    let after = lower[at + len..].chars().next();
    !before.is_some_and(|c| c.is_ascii_alphabetic())
        && !after.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '-'))
}

/// The word right after a label and its separator (`:`, `=`, spaces, an opening quote).
fn word_after_label(rest: &str) -> &str {
    let first = rest
        .trim_start_matches([':', '=', ' ', '\t', '|', '"', '\'', '`'])
        .split_whitespace()
        .next()
        .unwrap_or("");
    trim_value(first)
}

/// A secret after a label such as `password`, `pw:`, `passphrase is`, `sessionid=` or `password = "..."`,
/// or a long base64 value after `token`.
fn labelled_secret(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let plain = LABELS.iter().any(|label| {
        lower
            .match_indices(label)
            .filter(|(i, _)| whole_label(&lower, *i, label.len()))
            .any(|(i, _)| {
                let rest = &text[i + label.len()..];
                if secret_looking(word_after_label(rest)) {
                    return true;
                }
                // `password for the vault until Friday: X`, `passphrase is X`, `Passwort ... ist X`.
                let words: Vec<&str> = rest.split_whitespace().take(14).collect();
                words.windows(2).any(|w| {
                    let k = w[0].to_ascii_lowercase();
                    let marks = k.ends_with(':') || matches!(k.as_str(), "is" | "ist" | "=" | ":=");
                    marks && secret_looking(trim_value(w[1]))
                })
            })
    });
    plain
        || BLOB_LABELS.iter().any(|label| {
            lower
                .match_indices(label)
                .filter(|(i, _)| whole_label(&lower, *i, label.len()))
                .any(|(i, _)| long_base64(word_after_label(&text[i + label.len()..])))
        })
}

/// At least 40 characters of base64 with upper case, lower case and a digit: an encoded secret value.
fn long_base64(v: &str) -> bool {
    v.len() >= 40
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        && v.chars().any(|c| c.is_ascii_uppercase())
        && v.chars().any(|c| c.is_ascii_lowercase())
        && v.chars().any(|c| c.is_ascii_digit())
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

    #[test]
    fn half_a_private_key_marker_is_not_a_private_key() {
        for text in [
            "-----BEGIN PUBLIC KEY-----\nMFkw",
            "-----BEGIN CERTIFICATE-----",
            "ends with PRIVATE KEY-----",
        ] {
            let kind = credential_kind(text);
            assert_ne!(kind, Some("a private key"), "{text:?}");
        }
        assert_eq!(
            credential_kind("-----BEGIN RSA PRIVATE KEY-----"),
            Some("a private key")
        );
    }

    #[test]
    fn only_three_parts_starting_with_eyj_make_a_json_web_token() {
        let part = "abcdefghijkl";
        let cases = [
            (format!("eyJhbGciOi.{part}.{part}"), true),
            (format!("xyJhbGciOi.{part}.{part}"), false),
            (format!("eyJhbGciOi.{part}"), false),
            (format!("eyJhbGciOi.{part}.{part}.{part}"), false),
        ];
        for (text, jwt) in cases {
            assert_eq!(json_web_token(&text), jwt, "{text:?}");
        }
    }

    /// Obviously fake values built at runtime so no scanner sees a literal credential in the source.
    fn fake_password() -> String {
        ["Zq8", "rT2!", "vK9m", "_Lw4x"].concat()
    }

    #[test]
    fn a_password_inside_a_url_or_a_command_is_recognised() {
        let pw = fake_password();
        let cases = [
            format!("the db url is postgres://app:{pw}@db.internal.example:5432/app"),
            format!("redis is at redis://default:{pw}@cache.example.net:6379"),
            format!("Error: auth failed for postgres://svc:{pw}@10.0.3.4:5432/main"),
            format!("the failing curl:\ncurl -u deploy:{pw} https://artifacts.example.com/upload"),
            format!("curl --user deploy:{pw} https://x.example.com"),
            format!("mysql -h db.example.org -u root -p{pw} appdb"),
            format!("psql --password={pw} app"),
            "postgresql://reporter:%5A%71%38%72%54%32@203.0.113.40:5432/reports".to_string(),
        ];
        for text in cases {
            assert_eq!(credential_kind(&text), Some("a password"), "{text:?}");
        }
    }

    #[test]
    fn urls_and_commands_without_a_real_password_pass() {
        let cases = [
            "DATABASE_URL=postgres://localhost/app_test (no password, local dev only)",
            "postgres://user@host/db",
            "postgres://user:@host/db",
            "postgres://user:<password>@host/db",
            "postgres://user:${DB_PASSWORD}@host/db",
            "postgres://user:********@host/db",
            "postgres://user:password@host/db",
            "see https://example.com:8080/path for it",
            "curl -u deploy https://x.example.com",
            "curl -u deploy:$TOKEN https://x.example.com",
            "mysql -h db.example.org -u root -p appdb",
            "mysql -p",
            "run mysql -u root -p and type it at the prompt",
        ];
        for text in cases {
            assert_eq!(credential_kind(text), None, "{text:?}");
        }
    }

    #[test]
    fn an_npm_token_is_recognised() {
        let token = format!("npm_{}", "a1B2".repeat(9));
        assert_eq!(credential_kind(&token), Some("an npm token"));
        let line = format!("//registry.npmjs.org/:_authToken={token}");
        assert_eq!(credential_kind(&line), Some("an npm token"));
        assert_eq!(credential_kind("npm_config_cache is set"), None);
        assert_eq!(credential_kind("npm_short1234"), None);
    }

    #[test]
    fn a_keyword_labelled_secret_is_recognised() {
        let pw = fake_password();
        let hex = "9f2c".repeat(8);
        let cases = [
            format!("admin login for the staging dashboard: user ops, password {pw}"),
            format!("temporary password for the shared vault until Friday: {pw}"),
            format!("ssh in with the key I put at /tmp/k, passphrase is {pw}"),
            format!("pw: {pw}"),
            format!("terraform plan output:\n  + password = \"{pw}\"\n  + username = \"admin\""),
            format!("cookie for the admin session: sessionid={hex}; it's valid for a day"),
            format!("webhook signing secret: {hex}"),
            format!("Das Passwort für die Staging-Datenbank ist {pw}"),
            format!("DB_PASSWORD={pw}"),
            format!("PASSWORD: '{pw}'"),
            format!("| DB_PASSWORD | {pw} | prod |"),
            format!("[smtp]\nuser = \"alerts\"\npass = \"{pw}\""),
            format!(
                "webhook signing secret for the payments endpoint (we verify the HMAC with it): {hex}"
            ),
        ];
        for text in cases {
            assert_eq!(
                credential_kind(&text),
                Some("a labelled secret"),
                "{text:?}"
            );
        }
    }

    #[test]
    fn talk_about_passwords_without_one_passes() {
        let cases = [
            "password: ******** (masked in the log, as it should be)",
            "the password is required",
            "the password is in the vault",
            "password: <your password here>",
            "password = \"${DB_PASSWORD}\"",
            "password {{fake:password}}",
            "export GITHUB_TOKEN=${{ secrets.GITHUB_TOKEN }} in the workflow",
            "the TOTP secret goes in the authenticator app, never in chat",
            "rotate the passphrase every year",
            "pw: ...",
            "password reset link expired",
            "zq9-secret-marker-7731",
            "see client-secret-rotation-2026 for the plan",
            "the pass-through-2026 route",
            "password_hash: column added",
            "the token lives in the keychain under the item klatch-store; never paste it here",
            "test vector: HMAC-SHA256 of 'abc' with key 'key' is 9c196e32dc0175f86f4b1cb89289d6619de6bee699e4c378e68309ed97a1a6ab",
            "the release tarball sha256 is 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "the base64 of the test fixture is iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==; it's just the PNG header",
        ];
        for text in cases {
            assert_eq!(credential_kind(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_long_base64_value_labelled_token_or_secret_is_recognised() {
        let blob = format!("Qm{}", "x7Kp9Zr3Ld5Wn1Vb8Tc2Hg4Jf6Sa0Ye+/".repeat(2));
        for text in [
            format!(
                "kubectl get secret app -o yaml shows data.token: {blob} which decodes to the key"
            ),
            format!("secret = {blob}"),
        ] {
            assert_eq!(
                credential_kind(&text),
                Some("a labelled secret"),
                "{text:?}"
            );
        }
        assert_eq!(credential_kind("token: abcdefgh"), None);
    }
}
