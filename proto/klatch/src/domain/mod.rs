//! The domain module (the `klatch-core` crate before the crates became modules): the types and pure logic every other crate shares.
//! No I/O and no dependencies, so it can be tested without a database.

mod ask_bounds;
mod ask_cycle;
mod ask_find;
mod ask_refs;
mod ask_routing;
mod asker_gone;
mod asks;
mod asks_notes;
mod board;
mod briefing;
mod card;
mod cascade_judge;
mod claim_age;
mod convention;
mod convention_words;
mod could_answer;
mod credentials;
mod cycle_notice;
mod dm;
mod error;
mod hook;
mod hosted_judge;
mod judge;
mod judge_loop;
mod kind;
mod labels;
mod machine_checkpoint;
mod machine_status;
mod mediator;
mod merge_claim;
mod passive_claim;
pub(crate) mod pre_edit;
mod priority;
mod session_budget;
pub use merge_claim::{MERGE_CLAIM, MERGE_CLAIM_MAX_SECS, MERGE_PARK_MAX_SECS, is_merge_claim};
pub use passive_claim::{
    PASSIVE_CLAIM_SECS, PASSIVE_REASON, PassiveOutcome, PassiveRefusal, PassiveTaken, covers,
    never_claimed, passive_path, refusal_line,
};
pub use pre_edit::{pre_edit_refusal_line, pre_edit_warning};
mod merge_queue;
pub use merge_queue::{
    CHECKING_PREFIX, DROPPED_PREFIX, DropReason, HALTED_PREFIX, Halt, HaltReason,
    INTEGRATING_PREFIX, QUEUE_ENDED_SHOWN, QUEUE_WORDS, QueueEntry, QueueState, READY_PREFIX,
    REVERTED_PREFIX, merge_halt, merge_queue,
};
mod model;
pub(crate) mod parked;
mod pr_lifecycle;
mod receipt;
mod router;
mod scope;
mod stuck;
mod subagent_name;
mod summary;
mod tasks;
mod text;
mod toolguard;
#[cfg(test)]
mod toolguard_tests;
mod usage_stats;
mod wait_filter;
mod wake_cap;
mod wake_path;

pub use ask_bounds::{ASK_GIVE_UP_SECS, give_up_notes};
pub use ask_cycle::{AskCycle, ask_cycles, cycle_notes};
pub use ask_find::{AnsweredAsk, find_answered};
pub use ask_refs::{
    AskRef, KNOWN_KINDS, MAX_LABEL_BYTES, MAX_REFS, MAX_SNIPPET_BYTES, MAX_TOTAL_BYTES,
    MAX_VALUE_BYTES, RefProblem, check_refs, inert, named_room, ref_lines, refs_mark,
    valid_ref_kind,
};
pub use ask_routing::{
    AskRoute, Candidate, ESCALATED, LAPSED, ROUTE_MAX, ROUTED, RouteKind, RoutingCounts, TAKEN,
    affinity as route_affinity, pick as pick_routes,
};
pub use asker_gone::{Served, exited_asker_notes, gone_asker_notes, served_by};
pub use asks::{
    ASK_LEASE_DEFAULT_SECS, ASK_LEASE_MAX_SECS, ASK_PREFIX, ASK_WORDS, AskRoom, CLAIM_DEFAULT_SECS,
    CLAIM_MAX_SECS, Clarification, CloseOutcome, MovedDm, NO_CAPABLE_AFTER, OpenAsk, Outcome,
    OutcomeKind, ROOM_ACTIVE_SECS, STALE_AFTER_SECS, answer_target, claim_ttl, close_outcome,
    closed_target, is_ask, latest_close, moved_target, open_asks, parse_clarification,
    parse_outcome, reopen_target,
};
pub use asks_notes::{AsksView, NOTES_PER_KIND, SERVED_WINDOW, derived_asks_notes};
pub use board::{
    BOARD_LIST_LIMIT, BOARD_WORDS, Board, BoardMember, MEMBER_ACTIVE_SECS, MEMBER_RETENTION_SECS,
    MemberStatus, OFFER_PREFIX, OFFER_TTL_SECS, Offer, STATUS_PREFIX, WITHDRAW_PREFIX,
    ask_reach_note, current_offers, is_offer, nudge_skipped_note, render_board, render_who,
    short_dir_name, statuses,
};
pub use briefing::{
    BRIEFING_MAX_BYTES, BRIEFING_RECENT_MESSAGES, BriefingInput, Standing, StartSource, headline,
    other_rooms_line, render_briefing,
};
pub use card::{Card, card_of, render_card};
pub use machine_checkpoint::{
    MACHINE_MARK, MACHINE_MAX_BYTES, MachineCheckpoint, MachineState, TRAIL_PATH_MAX_BYTES,
    ToolClass, Trail, end_word, machine_body, repo_relative, status_word,
};
pub use machine_status::{
    MACHINE_STATUS_WINDOW_SECS, MachineClass, SESSION_END_STATUS, is_machine_status, machine_owned,
};
pub use parked::{PARKED_DEFAULT_SECS, PARKED_MAX_SECS, PARKED_PREFIX, Parked, parse_parked};
pub use pr_lifecycle::{MERGED_PREFIX, OpenPr, PR_PREFIX, PR_WORDS, is_merged_note, open_prs};
pub use tasks::{
    CHECKPOINT_PREFIX, Checkpoint, FINISHED_PREFIX, HANDOFF_PREFIX, TASK_HOLD_SECS, TASK_WORDS,
    Task, WORKING_PREFIX, open_tasks,
};
pub mod budget;
pub use cascade_judge::{
    CascadeJudge, CascadePolicy, Decided, FIRST_STAGE_REDACTION, GatedReader, Lean, Reader,
    Reading, ReadingTransport, Stage,
};
pub use claim_age::{AgedClaim, FIRST_MILESTONE_SECS, due_milestone, milestone};
pub use convention::{CONVENTION_MARKER, FEED_ROOM, convention, convention_in, subagent_brief};
pub use convention_words::{CONVENTION_WORDS, ConventionWord};
pub use could_answer::{Blocker, CouldAnswer};
pub use credentials::credential_kind;
pub use cycle_notice::CycleNotice;
pub use dm::{
    DM_NUDGE_MESSAGES, DM_NUDGE_THRESHOLDS, DM_NUDGE_WINDOW, DM_PREFIX, clarification_round,
    crossing_exchange, dm_nudge, dm_peer_in_room, dm_reply_hint, dm_room, exchange_run,
    move_to_dm_nudge,
};
pub use error::{Error, NEXT_STEP, Result};
pub use hook::{
    RoomDigest, cut_lines_to_fit, hook_context_json, hook_output_json, outside_view_line,
    printed_len, refused_lines_within, render_hook, render_hook_at, render_hook_within,
};
pub use hosted_judge::{HostedConfig, HostedJudge, HostedTransport, label_below_ceiling};
pub use judge::{Judge, NullJudge, ReviewItem, Verdict};
pub use judge_loop::{Breaker, Narrowed, RulesJudge};
pub use kind::{KIND_KEY, MemberKind};
#[cfg(kani)]
pub(crate) use labels::{ChainError, check_chain, default_cap_in, dominates_in, min_in};
pub use labels::{DEFAULT_LABELS, LabelChain};
pub use mediator::{MAX_PROPOSALS, Proposal, View as MediatorView, plan as plan_proposals};
pub use model::{
    Attribution, AuditRecord, Claim, ClaimChanges, CrossingStatus, HookRun, MemberInfo,
    MemberPlace, Message, Posted, Principal, RefusalRecord, ReleaseNote, ReleasedClaim,
};
pub use priority::{Importance, REORDERED_NOTE, is_reordered, order_by_importance};
pub use receipt::{
    MemberCursor, Receipt, Receipts, Taken, receipt_of, receipts_of, render_receipts, taken_of,
};
pub use router::{
    AskFacts, MAX_INVITES, Route, Rule, View as RouterView, plan as plan_routes, topic_room,
};
#[cfg(kani)]
pub(crate) use scope::scope_bytes_ok;
pub use scope::{
    MemberAttrs, SHARED_SCOPE, WORK_ITEM_NOUNS, canonical_claim_path, claim_scope, scopes_collide,
    valid_scope, work_item_name,
};
pub use session_budget::{BudgetLevel, PROMPT_BUDGET_BYTES, SESSION_BUDGET_BYTES};
pub use stuck::{
    LAPSE_RECORD_CAP, LAPSE_TOTAL_CAP, LAPSED_CLAIM_MAX, LAPSED_WINDOW_SECS, StuckEvent,
    YIELD_WINDOW_SECS, claim_label, stuck_events,
};
pub use subagent_name::invokes_klatch_unnamed;
pub use summary::{
    SUMMARY_LINE, SUMMARY_MAX_BYTES, SUMMARY_MAX_LINES, SUMMARY_ROOMS, SummaryRoom, render_summary,
};
#[cfg(kani)]
pub(crate) use text::safe_name_bytes;
pub use text::{
    MAX_PAYLOAD_BYTES, RESERVED_NAMES, flat, is_reserved_name, mentions, norm, overlaps,
    secs_between, valid_kind, valid_name,
};
#[cfg(kani)]
pub(crate) use toolguard::clean_bytes;
pub use toolguard::{
    Asker, EDIT_TOOLS, Held, ToolCall, ToolVerdict, clean_path, command_words, judge_tool,
    patch_targets,
};
pub use usage_stats::{
    AskCounts, ClaimCounts, ClaimHistory, Counts, MachineCheckpointCounts, MachineStatusCounts,
    MemberCounts, PassiveClaimCounts, RoomClass, RoomCount, StatsOptions, UsageStats, claim_class,
    day_of, median_and_p90, start_of_day, vocab,
};
pub use wait_filter::{Filter as WaitFilter, resolve_dm_targets, room_filters};
pub use wake_cap::{cut_to, fit_prefix, mentioning};
pub use wake_path::{PROMPTING_MODES, WakePath, wake_path};
