//! Grading judges (#914): a labelled synthetic corpus of candidate crossings and a runner that feeds each one to a
//! judge through the `Judge` seam and reports leaks, over-blocking, deferrals, per-verdict precision and recall,
//! latency, tokens and cost, calibration and a per-category breakdown. See `judge-eval/README.md` for the corpus and
//! the labelling rules, and the design item 207 for how to run each judge.
//!
//! Everything here is deterministic without a model: the metric code, the loader and the claude judge (behind a
//! process-runner trait) are tested with fakes. Running a real model is a manual command.

pub mod cascade;
pub mod cascade_sim;
pub mod claude;
pub mod cli;
pub mod corpus;
pub mod interval;
pub mod jev;
pub mod judges;
pub mod metrics;
pub mod report;
pub mod run;
pub mod semantics;
