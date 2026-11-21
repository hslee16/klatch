
Short-lived credentials (#161): an issued credential lives for an idle timeout (`--ttl`, 3600 seconds by default) that
each successful request pushes out again, up to a hard cap from issue (`--max-life`, 24 hours by default, at most 30
days, `MAX_CREDENTIAL_TTL_SECS`) that renewal never passes. They are issued only by the human at a launcher, revoked at
once by `klatch worker revoke` or by the worker ending its own session, and rotated by revoke and reissue
(`worker_credentials.rs`, `credential_renewal.rs`). A credential issued before schema 31 is never renewed.
