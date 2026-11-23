//! `klatch-faults [--seed N] [--rounds R] [--workers W] [--ops M] [--no-rotate]`: the multi-process fault tester (#625). It prints
//! its seed first, so a failing run can be repeated with `--seed`. Run by hand; it is not part of CI or the gate.

use klatch_bench::mp_run::{Config, RoundReport, run_round};

fn number(args: &mut impl Iterator<Item = String>, flag: &str) -> u64 {
    args.next()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| usage_error(&format!("{flag} needs a number")))
}

/// Say what is wrong with the command line and stop.
fn usage_error(what: &str) -> ! {
    eprintln!("klatch-faults: {what}");
    std::process::exit(2);
}

/// The plan the command line asks for.
fn parse_config(mut args: impl Iterator<Item = String>) -> Config {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64);
    let mut cfg = Config {
        seed: now,
        rounds: 4,
        workers: 4,
        ops: 60,
        rotate: true,
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seed" => cfg.seed = number(&mut args, "--seed"),
            "--rounds" => cfg.rounds = number(&mut args, "--rounds") as usize,
            "--workers" => cfg.workers = number(&mut args, "--workers") as usize,
            "--no-rotate" => cfg.rotate = false,
            "--ops" => cfg.ops = number(&mut args, "--ops") as usize,
            other => usage_error(&format!("unknown argument {other}")),
        }
    }
    cfg
}

/// Print one round; whether it broke what klatch promises.
fn print_round(r: &RoundReport) -> bool {
    let verdict = if r.violations.is_empty() {
        "held"
    } else {
        "BROKEN"
    };
    println!(
        "round {}: {} processes, {} killed, {} posts acknowledged, plaintext start {}: {verdict}",
        r.round, r.started, r.killed, r.acked, r.migrated
    );
    for v in &r.violations {
        println!("  violation: {v}");
    }
    for (what, n) in &r.refusals {
        println!("  refused {n}x: {what}");
    }
    !r.violations.is_empty()
}

fn main() {
    let cfg = parse_config(std::env::args().skip(1));
    let bin = klatch_bench::find_binary().unwrap_or_else(|e| usage_error(&e));
    println!(
        "klatch-faults seed {} (repeat the plan with --seed {}; how the processes interleave still varies)",
        cfg.seed, cfg.seed
    );
    let mut failed = false;
    for round in 0..cfg.rounds {
        failed |= match run_round(&bin, &cfg, round) {
            Ok(r) => print_round(&r),
            Err(e) => {
                println!("round {round}: the tester itself failed: {e}");
                true
            }
        };
    }
    if failed {
        std::process::exit(1);
    }
}
