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
use crate::tls_admit::SessionSlots;
use crate::tls_limits::{Handshakes, Limits};
use crate::tls_reload::{self, Current};
use crate::wire_http::{self, Shared};
use anyhow::{Context, Result, bail};
use klatch::gateway::SharedGateway;
use klatch::session::SessionConfig;
use klatch::session::policy::TlsBind;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// What `klatch-server https` was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub listen: String,
    pub max_connections: usize,
    pub files: TlsFiles,
    pub allow_off_loopback: bool,
    pub limits: Limits,
    /// Origins the WebSocket route lets through (`--ws-allow-origin`, #112).
    pub ws: crate::wire_ws_caps::Options,
}

/// The longest handshake deadline: a client that needs more than a minute to say hello is not one to wait for.
pub const MAX_HANDSHAKE_SECS: u64 = 60;
/// The longest stall time: a day. Larger values only disable the limit.
pub const MAX_STALL_SECS: u64 = 86_400;

/// A whole number of seconds from 1 to `max`; the refusal names the flag and the bound.
fn seconds(flag: &str, text: &str, max: u64) -> Result<u64> {
    text.parse()
        .ok()
        .filter(|n| (1..=max).contains(n))
        .with_context(|| {
            format!("{flag} is a whole number of seconds from 1 to {max}, not {text:?}")
        })
}

/// Whether this process may take limits below the shipped minimums: only a debug build (what `cargo test` builds) and
/// only with `KLATCH_TEST_SHORT_LIMITS` set. A release binary has no such knob: the check is compiled out.
fn short_limits_knob() -> bool {
    cfg!(debug_assertions) && std::env::var_os("KLATCH_TEST_SHORT_LIMITS").is_some()
}

/// Read the options after `https`. The address has no default, and each of the three files is required: a server never
/// picks a place or a certificate by itself, and has no mode without a client CA.
pub fn parse(args: &[&str]) -> Result<Options> {
    parse_with(args, short_limits_knob())
}

/// The flag values as given, checked together once every option has been read (the bounds depend on each other).
#[derive(Default)]
struct Raw<'a> {
    max_handshakes: Option<&'a str>,
    per_source: Option<&'a str>,
    handshake: Option<&'a str>,
    stall: Option<&'a str>,
}

/// The smallest stall time: twice the event stream's heartbeat. A debug build with the test knob may go down to one.
fn stall_floor(short_limits: bool) -> u64 {
    if short_limits && cfg!(debug_assertions) {
        1
    } else {
        2 * SessionConfig::default().heartbeat.as_secs()
    }
}

/// The limits the options ask for, within their bounds: a handshake of 1 to 60 seconds, a stall of 1 to 86400 seconds and
/// at least twice the event stream's heartbeat (a quieter limit would cut a stream that is only waiting for its next
/// heartbeat; `short_limits` lowers that floor, for tests), and no more handshakes than connections.
fn limits_of(raw: &Raw, max_connections: usize, short_limits: bool) -> Result<Limits> {
    let mut limits = Limits::defaults(max_connections);
    if let Some(text) = raw.max_handshakes {
        limits.max_handshakes = text
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=max_connections).contains(n))
            .with_context(|| {
                format!("--max-handshakes is a positive number up to --max-connections ({max_connections}), not {text:?}")
            })?;
    }
    if let Some(text) = raw.per_source {
        limits.per_source = text
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=limits.max_handshakes).contains(n))
            .with_context(|| {
                format!("--handshakes-per-source is a positive number up to --max-handshakes ({}), not {text:?}", limits.max_handshakes)
            })?;
    }
    if let Some(text) = raw.handshake {
        limits.handshake =
            Duration::from_secs(seconds("--handshake-secs", text, MAX_HANDSHAKE_SECS)?);
    }
    if let Some(text) = raw.stall {
        let secs = seconds("--stall-secs", text, MAX_STALL_SECS)?;
        let floor = stall_floor(short_limits);
        if secs < floor {
            bail!(
                "--stall-secs {secs} is below twice the event stream heartbeat ({floor} seconds): a stream that is only waiting for its next heartbeat would be cut; use {floor} or more"
            );
        }
        limits.stall = Duration::from_secs(secs);
    }
    Ok(limits)
}

/// [`parse`] with the test knob given.
pub fn parse_with(args: &[&str], short_limits: bool) -> Result<Options> {
    let (mut listen, mut cert, mut key, mut ca) = (None, None, None, None);
    let mut crl: Option<PathBuf> = None;
    let mut intermediates: Option<PathBuf> = None;
    let mut max_connections = wire_http::DEFAULT_MAX_CONNECTIONS;
    let mut allow_off_loopback = false;
    let mut raw = Raw::default();
    let mut ws = crate::wire_ws_caps::Raw::default();
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
            "--tls-client-crl" => {
                let path = PathBuf::from(value("--tls-client-crl")?);
                if crl.replace(path).is_some() {
                    bail!(
                        "--tls-client-crl is given more than once: name one file (it may hold several CRLs)"
                    );
                }
            }
            "--tls-client-intermediates" => {
                let path = PathBuf::from(value("--tls-client-intermediates")?);
                if intermediates.replace(path).is_some() {
                    bail!(
                        "--tls-client-intermediates is given more than once: name one file (it may hold several certificates)"
                    );
                }
            }
            "--allow-off-loopback" => allow_off_loopback = true,
            flag @ ("--ws-allow-origin"
            | "--ws-max-per-member"
            | "--ws-max-age-secs"
            | "--stream-max-per-member"
            | "--stream-max-age-secs") => {
                ws.take(flag, value(flag)?)?;
            }
            "--max-handshakes" => raw.max_handshakes = Some(value("--max-handshakes")?),
            "--handshakes-per-source" => {
                raw.per_source = Some(value("--handshakes-per-source")?);
            }
            "--handshake-secs" => raw.handshake = Some(value("--handshake-secs")?),
            "--stall-secs" => raw.stall = Some(value("--stall-secs")?),
            "--max-connections" => {
                max_connections = value("--max-connections")?
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .context("--max-connections is a positive number")?;
            }
            other => bail!("unknown option {} for https", klatch::domain::shown(other)),
        }
    }
    let limits = limits_of(&raw, max_connections, short_limits)?;
    if intermediates.is_some() && ca.is_none() {
        bail!(
            "--tls-client-intermediates needs --tls-client-ca: the file only matches the issuers of CRLs of a client CA, and mutual TLS has no mode without one"
        );
    }
    if intermediates.is_some() && crl.is_none() {
        bail!(
            "--tls-client-intermediates needs --tls-client-crl: it only matches the issuers of CRLs, so without a CRL it would do nothing"
        );
    }
    if crl.is_some() && ca.is_none() {
        bail!(
            "--tls-client-crl needs --tls-client-ca: a CRL revokes client certificates of a client CA, and mutual TLS has no mode without one"
        );
    }
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
            client_crl: crl,
            client_intermediates: intermediates,
        },
        allow_off_loopback,
        ws: ws.finish(max_connections, short_limits)?,
    })
}

/// The readiness line, as one complete write (see `wire_http::announce_listening`).
fn announce_listening(out: &mut dyn Write, addr: &dyn std::fmt::Display, opts: &Options) {
    let line = format!(
        "klatch-server: https listening on {addr} (mutual TLS, {}, up to {} connections), GET {}, POST {}, POST {} (MCP) and GET {} (WebSocket)\n",
        if opts.allow_off_loopback {
            "off loopback allowed"
        } else {
            "loopback only"
        },
        opts.max_connections,
        wire_http::STREAM_PATH,
        wire_http::FRAMES_PATH,
        crate::mcp_http::PATH,
        crate::wire_ws::PATH
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
    let (config, crl_due) = tls::build_config_at(&opts.files, roots, crate::tls_crl::Stage::Start)?;
    let gateway: SharedGateway = klatch::member::spawn_shared_gateway()
        .context("the server cannot open the store, so it does not start")?;
    let listener = loopback::bind_mutual_tls(&opts.listen, policy)?;
    crate::daemon::install_signal_handlers_with_hangup();
    if let Some(path) = &opts.files.client_crl {
        eprintln!("{}", crate::tls_crl::start_line(path));
    }
    announce_listening(&mut io::stderr().lock(), &listener.local_addr()?, opts);
    let mut shared = Shared::new(gateway.session_backend(), SessionConfig::default());
    shared.ws.apply(&opts.ws, opts.max_connections);
    let handshakes = Handshakes::default();
    let sessions = SessionSlots::new(opts.max_connections);
    // SIGHUP reloads the TLS files on a thread of its own (`tls_reload.rs`); each connection takes the configuration
    // current when its handshake starts and keeps it
    let current = Current::new(config);
    crate::forwarded_report::while_reporting(&shared.forwarded, &shared.backend, || {
        watched_serve(
            &crate::daemon::STOP,
            || {
                tls_reload::watch(
                    &crate::daemon::STOP,
                    &crate::daemon::RELOAD,
                    opts,
                    &current,
                    crl_due,
                )
            },
            || {
                // the descriptors and threads of the process are bounded by the sessions plus the handshake pool; the session cap
                // itself is `sessions` (a connection takes a place there only after its handshake), so a flood of handshakes
                // cannot take the place of a session
                loopback::serve_concurrent(
                    &listener,
                    &crate::daemon::STOP,
                    opts.max_connections
                        .saturating_add(opts.limits.max_handshakes),
                    loopback::Callbacks {
                        handler: &|sock: Stream| {
                            let config = current.get();
                            let at = Listener {
                                config: &config,
                                limits: &opts.limits,
                                handshakes: &handshakes,
                                sessions: &sessions,
                            };
                            tls::serve_tls(sock, &at, |app| {
                                wire_http::serve_connection(app, &shared)
                            })
                        },
                        excess: &drop,
                    },
                );
            },
        );
    });
    drop(shared);
    drop(gateway);
    eprintln!("klatch-server: https stopped");
    Ok(())
}

/// Run `watch` on a thread of its own and `serve` on this one, and set `stop` when `serve` returns, however it returned (a
/// signal, or the accept loop could not start): the watcher ends with it, and this returns only when both have ended.
fn watched_serve(stop: &AtomicBool, watch: impl FnOnce() + Send, serve: impl FnOnce()) {
    std::thread::scope(|scope| {
        scope.spawn(watch);
        serve();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    /// #1637: when the accept loop returns on its own (`set_nonblocking` failed: it cannot be triggered here, so the
    /// serve closure returns at once), the stop flag is set and the watcher, which runs until it, ends. A watcher that
    /// was never told would spin to the safety limit and report it.
    #[test]
    fn the_watcher_ends_when_the_accept_loop_returns_by_itself() {
        let stop = AtomicBool::new(false);
        let told = AtomicBool::new(false);
        let started = Instant::now();
        watched_serve(
            &stop,
            || {
                while started.elapsed() < Duration::from_secs(10) {
                    if stop.load(Ordering::SeqCst) {
                        told.store(true, Ordering::SeqCst);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            },
            || {},
        );
        assert!(
            told.load(Ordering::SeqCst),
            "the watcher was never told to stop"
        );
        assert!(stop.load(Ordering::SeqCst));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

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
    fn the_limits_have_safe_defaults_and_each_can_be_set_within_its_bounds() {
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
            "90",
        ]))
        .unwrap()
        .limits;
        assert_eq!(
            (
                set.max_handshakes,
                set.handshake.as_secs(),
                set.stall.as_secs()
            ),
            (3, 2, 90)
        );
        let small = parse(&with(&["ca.pem", "--max-connections", "3"])).unwrap();
        assert_eq!(small.limits.max_handshakes, 1);
        assert_eq!(small.limits.per_source_within(), 1);
    }

    #[test]
    fn the_share_of_one_source_defaults_to_two_and_is_bounded_by_the_pool() {
        let d = parse(&with(&["ca.pem"])).unwrap().limits;
        assert_eq!(d.per_source, 2);
        let set = parse(&with(&["ca.pem", "--handshakes-per-source", "5"]));
        assert_eq!(set.unwrap().limits.per_source, 5);
        // the pool is 8 by default, 3 when asked: the bound follows the pool, whatever the order of the options
        let e = refusal(
            &["--handshakes-per-source", "4", "--max-handshakes", "3"],
            false,
        );
        assert!(
            e.contains("--handshakes-per-source") && e.contains("--max-handshakes"),
            "{e}"
        );
        for bad in ["0", "x", "-1", "9", "18446744073709551615"] {
            let e = refusal(&["--handshakes-per-source", bad], false);
            assert!(e.contains("--handshakes-per-source"), "{bad}: {e}");
        }
    }

    fn refusal(extra: &[&'static str], short_limits: bool) -> String {
        let mut args = with(&["ca.pem"]);
        args.extend_from_slice(extra);
        parse_with(&args, short_limits).unwrap_err().to_string()
    }

    #[test]
    fn a_value_that_is_not_a_positive_number_or_is_over_its_bound_is_refused_naming_the_flag_and_the_bound()
     {
        for bad in ["0", "x", "-1", "18446744073709551615", "61"] {
            let e = refusal(&["--handshake-secs", bad], false);
            assert!(
                e.contains("--handshake-secs") && e.contains("60"),
                "{bad}: {e}"
            );
        }
        for bad in ["0", "x", "18446744073709551615", "86401"] {
            let e = refusal(&["--stall-secs", bad], false);
            assert!(
                e.contains("--stall-secs") && e.contains("86400"),
                "{bad}: {e}"
            );
        }
        for bad in ["0", "x", "18446744073709551615", "33"] {
            let e = refusal(&["--max-handshakes", bad], false);
            assert!(
                e.contains("--max-handshakes") && e.contains("--max-connections"),
                "{bad}: {e}"
            );
        }
        // the bound of --max-handshakes is the cap asked for, whatever the order of the options
        let e = refusal(&["--max-handshakes", "9", "--max-connections", "8"], false);
        assert!(e.contains("--max-handshakes"), "{e}");
        assert!(
            parse(&with(&[
                "ca.pem",
                "--max-handshakes",
                "8",
                "--max-connections",
                "8"
            ]))
            .is_ok()
        );
        for ok in ["1", "60"] {
            assert!(
                parse(&with(&["ca.pem", "--handshake-secs", ok])).is_ok(),
                "{ok}"
            );
        }
        assert!(parse(&with(&["ca.pem", "--stall-secs", "86400"])).is_ok());
    }

    #[test]
    fn a_stall_time_under_twice_the_heartbeat_needs_the_test_knob_and_only_a_debug_build_has_it() {
        let heartbeat = SessionConfig::default().heartbeat.as_secs();
        let too_short = (2 * heartbeat - 1).to_string();
        let too_short: &'static str = Box::leak(too_short.into_boxed_str());
        let e = refusal(&["--stall-secs", too_short], false);
        assert!(
            e.contains("heartbeat") && e.contains(&(2 * heartbeat).to_string()),
            "{e}"
        );
        let exact: &'static str = Box::leak((2 * heartbeat).to_string().into_boxed_str());
        assert!(parse_with(&with(&["ca.pem", "--stall-secs", exact]), false).is_ok());
        // the knob lowers only that minimum, and only where the build has it
        let with_knob = parse_with(&with(&["ca.pem", "--stall-secs", "2"]), true);
        assert_eq!(with_knob.is_ok(), cfg!(debug_assertions));
    }
}

#[cfg(test)]
mod echo_tests {
    use super::*;

    /// #1776: an unknown option the operator typed is echoed in the refusal; a terminal escape, a bidi override or a
    /// zero-width character in it must not reach the terminal raw.
    #[test]
    fn an_unknown_option_is_echoed_escaped() {
        let e = format!(
            "{:#}",
            parse(&["--bad\u{1b}[2J\u{202e}\u{200b}"])
                .err()
                .expect("an unknown option is refused")
        );
        assert!(e.contains("\\u{1b}"), "{e}");
        assert!(
            !e.chars().any(|c| c.is_control()) && !e.contains(['\u{202e}', '\u{200b}']),
            "{e:?}"
        );
    }
}
