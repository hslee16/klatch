//! `klatch-server https --listen IP:PORT --tls-cert PEM --tls-key PEM --tls-client-ca PEM` (#108, epic #13, gate #157):
//! the wire of `wire_http.rs` (the same routes, the same events, resume by `seq`) behind a mutual TLS listener. It is not
//! a second protocol and not a second set of handlers: after the handshake (`tls.rs`) each connection goes to
//! `wire_http::serve_connection`, so a frame means here what it means over loopback HTTP, on stdio and in the daemon.
//!
//! - **The gate is the handshake.** A client needs a certificate from the CA file you name. Without one, with one from
//!   another CA, or with an expired one, it never reaches HTTP. There is no plaintext mode and no option to skip a check.
//! - **The caller is still a worker credential.** A client certificate gates the port and names no member: every route
//!   still needs `Authorization: Bearer TOKEN` and `X-Klatch-Member`, checked live. The certificate's subject is not read.
//! - **Where it listens.** Loopback unless `--allow-off-loopback` is given (off by default). The policy that decides is
//!   `klatch::session::policy::check_mutual_tls_bind`; plaintext off loopback still has no way to start.
//! - **Order.** The options and the files are checked (a refusal ends the process before the store is touched), then
//!   the store is opened, then the address is bound.
//!
//! The key file must be mode 0600 and the server never prints key material (`tls.rs`). Limits are those of
//! `wire_http` (a connection over `--max-connections` is closed without an answer: there is nothing to say in plaintext
//! on a TLS port), and a client that stalls in the handshake holds a connection slot for at most five seconds.
//!
//! This module names no socket type and no TLS type: the listener comes from `loopback`, the handshake from `tls`.

use crate::loopback::{self, Stream};
use crate::tls::{self, Listener, TlsFiles};
use crate::tls_limits::{Handshakes, Limits};
use crate::wire_http::{self, Shared};
use anyhow::{Context, Result, bail};
use klatch::gateway::SharedGateway;
use klatch::session::SessionConfig;
use klatch::session::policy::TlsBind;
use std::io::{self, Write};
use std::path::PathBuf;

/// What `klatch-server https` was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub listen: String,
    pub max_connections: usize,
    pub files: TlsFiles,
    pub allow_off_loopback: bool,
    pub limits: Limits,
}

/// A positive whole number (an option's value).
fn positive(text: &str) -> Result<u64> {
    text.parse()
        .ok()
        .filter(|n| *n > 0)
        .with_context(|| format!("{text:?} is not a positive number"))
}

/// Read the options after `https`. The address has no default, and each of the three files is required: a server never
/// picks a place or a certificate by itself, and has no mode without a client CA.
pub fn parse(args: &[&str]) -> Result<Options> {
    let (mut listen, mut cert, mut key, mut ca) = (None, None, None, None);
    let mut max_connections = wire_http::DEFAULT_MAX_CONNECTIONS;
    let mut allow_off_loopback = false;
    let (mut max_handshakes, mut handshake, mut stall) = (None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| -> Result<&str> {
            it.next()
                .copied()
                .with_context(|| format!("{name} needs a value"))
        };
        match *a {
            "--listen" => listen = Some(value("--listen")?.to_string()),
            "--tls-cert" => cert = Some(PathBuf::from(value("--tls-cert")?)),
            "--tls-key" => key = Some(PathBuf::from(value("--tls-key")?)),
            "--tls-client-ca" => ca = Some(PathBuf::from(value("--tls-client-ca")?)),
            "--allow-off-loopback" => allow_off_loopback = true,
            "--max-handshakes" => max_handshakes = Some(positive(value("--max-handshakes")?)?),
            "--handshake-secs" => handshake = Some(positive(value("--handshake-secs")?)?),
            "--stall-secs" => stall = Some(positive(value("--stall-secs")?)?),
            "--max-connections" => {
                max_connections = value("--max-connections")?
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .context("--max-connections is a positive number")?;
            }
            other => bail!("unknown option {other} for https"),
        }
    }
    let mut limits = Limits::defaults(max_connections);
    limits.max_handshakes = max_handshakes.map_or(limits.max_handshakes, |n| n as usize);
    limits.handshake = handshake.map_or(limits.handshake, std::time::Duration::from_secs);
    limits.stall = stall.map_or(limits.stall, std::time::Duration::from_secs);
    Ok(Options {
        limits,
        listen: listen.context("https needs --listen IP:PORT")?,
        max_connections,
        files: TlsFiles {
            cert: cert.context("https needs --tls-cert (the server certificate, PEM)")?,
            key: key.context("https needs --tls-key (the server's private key, PEM, mode 0600)")?,
            client_ca: ca.context(
                "https needs --tls-client-ca (a CA file, PEM, for client certificates): mutual TLS has no mode without one",
            )?,
        },
        allow_off_loopback,
    })
}

/// The readiness line, as one complete write (see `wire_http::announce_listening`).
fn announce_listening(out: &mut dyn Write, addr: &dyn std::fmt::Display, opts: &Options) {
    let line = format!(
        "klatch-server: https listening on {addr} (mutual TLS, {}, up to {} connections), GET {}, POST {} and POST {} (MCP)\n",
        if opts.allow_off_loopback {
            "off loopback allowed"
        } else {
            "loopback only"
        },
        opts.max_connections,
        wire_http::STREAM_PATH,
        wire_http::FRAMES_PATH,
        crate::mcp_http::PATH
    );
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

/// Run the server until SIGTERM or SIGINT. The order: the files are read and checked (the client roots first, so the
/// policy sees whether a client CA really exists; the key file's checks), then the address is checked by the policy,
/// then the store is opened, then the address is bound. A refusal ends the process before the store is touched.
pub fn run(opts: &Options) -> Result<()> {
    let roots = tls::client_roots(&opts.files.client_ca)?;
    let policy = TlsBind {
        client_ca_configured: !roots.is_empty(),
        allow_off_loopback: opts.allow_off_loopback,
    };
    loopback::check_mutual_tls_addr(&opts.listen, policy)?;
    let config = tls::build_config(&opts.files, roots)?;
    let gateway: SharedGateway = klatch::member::spawn_shared_gateway()
        .context("the server cannot open the store, so it does not start")?;
    let listener = loopback::bind_mutual_tls(&opts.listen, policy)?;
    crate::daemon::install_signal_handlers();
    announce_listening(&mut io::stderr().lock(), &listener.local_addr()?, opts);
    let shared = Shared::new(gateway.session_backend(), SessionConfig::default());
    let handshakes = Handshakes::default();
    let at = Listener {
        config: &config,
        limits: &opts.limits,
        handshakes: &handshakes,
    };
    loopback::serve_concurrent(
        &listener,
        &crate::daemon::STOP,
        opts.max_connections,
        loopback::Callbacks {
            handler: &|sock: Stream| {
                tls::serve_tls(sock, &at, |app| wire_http::serve_connection(app, &shared))
            },
            excess: &drop,
        },
    );
    drop(shared);
    drop(gateway);
    eprintln!("klatch-server: https stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [&str; 7] = [
        "--listen",
        "127.0.0.1:0",
        "--tls-cert",
        "c.pem",
        "--tls-key",
        "k.pem",
        "--tls-client-ca",
    ];

    fn with(extra: &[&'static str]) -> Vec<&'static str> {
        let mut v = ALL.to_vec();
        v.extend_from_slice(extra);
        v
    }

    #[test]
    fn every_file_and_the_address_are_required_and_a_client_ca_has_no_default() {
        assert!(parse(&with(&["ca.pem"])).is_ok());
        for drop in 0..4 {
            let mut args = with(&["ca.pem"]);
            args.drain(drop * 2..drop * 2 + 2);
            assert!(parse(&args).is_err(), "without option {drop}");
        }
        assert!(parse(&ALL).is_err(), "--tls-client-ca needs a value");
    }

    #[test]
    fn the_off_loopback_allowance_is_off_unless_given_and_no_other_switch_exists() {
        assert!(!parse(&with(&["ca.pem"])).unwrap().allow_off_loopback);
        assert!(
            parse(&with(&["ca.pem", "--allow-off-loopback"]))
                .unwrap()
                .allow_off_loopback
        );
        for flag in ["--insecure", "--no-client-auth", "--plaintext"] {
            assert!(parse(&with(&["ca.pem", flag])).is_err(), "{flag}");
        }
    }

    #[test]
    fn the_limits_have_safe_defaults_and_each_can_be_set() {
        let d = parse(&with(&["ca.pem"])).unwrap().limits;
        assert_eq!(d, Limits::defaults(wire_http::DEFAULT_MAX_CONNECTIONS));
        assert!(d.max_handshakes < wire_http::DEFAULT_MAX_CONNECTIONS);
        let set = parse(&with(&[
            "ca.pem",
            "--max-connections",
            "8",
            "--max-handshakes",
            "3",
            "--handshake-secs",
            "2",
            "--stall-secs",
            "9",
        ]))
        .unwrap()
        .limits;
        assert_eq!(
            (
                set.max_handshakes,
                set.handshake.as_secs(),
                set.stall.as_secs()
            ),
            (3, 2, 9)
        );
        // the cap follows --max-connections when it is not given
        let small = parse(&with(&["ca.pem", "--max-connections", "3"])).unwrap();
        assert_eq!(small.limits.max_handshakes, 1);
        for bad in ["0", "x", "-1"] {
            assert!(
                parse(&with(&["ca.pem", "--max-handshakes", bad])).is_err(),
                "{bad}"
            );
            assert!(
                parse(&with(&["ca.pem", "--stall-secs", bad])).is_err(),
                "{bad}"
            );
        }
    }
}
