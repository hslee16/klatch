//! Waiting: `wait`, `wake` and the checks a waiting helper runs.

use super::*;

/// `klatch wait` and `klatch wake`.
pub(super) fn run_wait(ctx: &Ctx, cmd: WaitCmd) -> Result<()> {
    let t = ctx.t;
    match cmd {
        WaitCmd::Wait {
            rooms,
            timeout,
            only,
            from,
            peek,
        } => {
            let global = klatch::domain::WaitFilter {
                only,
                from: from.into_iter().collect(),
            };
            let rooms =
                klatch::domain::room_filters(&rooms, &global).map_err(|e| anyhow::anyhow!(e))?;
            wait(
                t,
                &WaitArgs {
                    rooms: &rooms,
                    timeout,
                    peek,
                    caller: (ctx.me, ctx.now),
                },
            )?
        }
        WaitCmd::Wake { timeout } => wake(t, (ctx.me, timeout))?,
    }
    Ok(())
}

/// The longest one `wait` may block, so an agent looks at its own state at least this often.
pub(crate) const MAX_WAIT_SECONDS: u64 = 300;

/// The timeout to use and whether the request had to be clamped.
pub(crate) fn effective_timeout(requested: u64) -> (u64, bool) {
    if requested > MAX_WAIT_SECONDS {
        (MAX_WAIT_SECONDS, true)
    } else {
        (requested, false)
    }
}

/// Without a filter or `--peek` a wait consumes what it returns, like a blocking read. With either, it leaves
/// messages unread (a later read or the hook still shows them) and remembers what it has looked at so it does
/// not wake for the same message twice.
pub(super) fn look(
    t: &dyn Transport,
    room: &str,
    filter: &klatch::domain::WaitFilter,
    peek: bool,
) -> Result<Vec<klatch::domain::Message>> {
    if filter.is_open() && !peek {
        return Ok(t.read(room, true)?);
    }
    let seen = t.wait_poll(room)?;
    if let Some(last) = seen.iter().map(|m| m.seq).max() {
        t.wait_seen(room, last)?;
    }
    Ok(seen.into_iter().filter(|m| filter.accepts(m)).collect())
}

/// What one `wait` call was asked: the rooms (each with its wake filter), the timeout, whether it only peeks, and
/// who is waiting and when it started (the caller's own asks are named when it times out, #200).
pub(super) struct WaitArgs<'a> {
    pub(super) rooms: &'a [(String, klatch::domain::WaitFilter)],
    pub(super) timeout: u64,
    pub(super) peek: bool,
    pub(super) caller: (&'a str, i64),
}

/// What a wait that timed out adds to the marker: the caller's own asks that have gone unanswered past the bound
/// (#200). A room that cannot be read adds nothing: a note never hides the marker.
pub(super) fn timeout_notes(t: &dyn Transport, a: &WaitArgs, waited: u64) -> Vec<String> {
    let (me, started) = a.caller;
    let now = started + waited as i64;
    a.rooms
        .iter()
        .filter_map(|(room, _)| t.open_asks(room).ok())
        .flat_map(|asks| klatch::domain::give_up_notes(&asks, me, now))
        .collect()
}

/// Print the messages that woke a wait; true when there were any.
pub(super) fn print_woken(t: &dyn Transport, a: &WaitArgs) -> Result<bool> {
    for (room, filter) in a.rooms {
        let found = look(t, room, filter, a.peek)?;
        let capped = klatch::domain::cap_wake(
            &found,
            a.caller.0,
            klatch::domain::budget::DIGEST_MAX_MESSAGES,
        );
        for m in &capped.shown {
            println!(
                "[{room} {} {} {}] {}{}",
                m.seq,
                m.ts,
                m.sender_label(),
                flat(&m.body),
                handoff_suffix(m)
            );
        }
        if capped.omitted > 0 {
            println!(
                "[{room}: {} more messages not shown, the ones addressed to you first; run `klatch history {room}` for them]",
                capped.omitted
            );
        }
        if !found.is_empty() {
            eprintln!("{DATA_RULE}");
            return Ok(true);
        }
    }
    Ok(false)
}

/// How many recent messages a waiting helper reads to find the asks it answered (#226).
pub(super) const SERVED_WINDOW: usize = 200;

/// The asks the caller `me` serves in `room`, from its recent messages and its claims. A room that cannot be read
/// serves nothing.
pub(super) fn served_in(
    t: &dyn Transport,
    me: &str,
    room: &str,
    now: i64,
) -> Result<Vec<klatch::domain::Served>> {
    let range = klatch::gateway::HistoryRange {
        since: None,
        last: Some(SERVED_WINDOW),
    };
    let messages = t.history(room, range)?.messages;
    Ok(klatch::domain::served_by(
        &messages,
        (&t.open_asks(room)?, &t.claims(room)?),
        (me, now),
    ))
}

/// The askers among `served` whose process has exited, by the liveness probe (#790, DESIGN item 96). An asker the
/// probe cannot say anything about (no session, an unreadable record, no `ps`) is not in the list: unknown counts
/// as alive.
pub(super) fn exited_askers(
    t: &dyn Transport,
    room: &str,
    served: &[klatch::domain::Served],
) -> Vec<String> {
    let mut exited: Vec<String> = Vec::new();
    for s in served {
        let known = exited.contains(&s.asker);
        if !known && t.process_exited(room, &s.asker).unwrap_or(false) {
            exited.push(s.asker.clone());
        }
    }
    exited
}

/// The askers the caller `me` serves in `room` that are gone, as lines (#226, DESIGN item 89; an exited process
/// is named at once, #790). A room that cannot be read adds nothing.
pub(super) fn gone_askers_in(
    t: &dyn Transport,
    me: &str,
    room: &str,
    now: i64,
) -> Result<Vec<String>> {
    let served = served_in(t, me, room, now)?;
    if served.is_empty() {
        return Ok(Vec::new());
    }
    let exited = exited_askers(t, room, &served);
    Ok(klatch::domain::gone_asker_notes(
        &served,
        &t.who(room)?,
        &exited,
        now,
    ))
}

/// The lines for askers whose process has exited, and only those: the check a waiting helper runs every few
/// seconds (#790). Nothing is read but the room and the probe when no asker's process is gone.
pub(super) fn exited_askers_in(
    t: &dyn Transport,
    me: &str,
    room: &str,
    now: i64,
) -> Result<Vec<String>> {
    let served = served_in(t, me, room, now)?;
    let exited = exited_askers(t, room, &served);
    if exited.is_empty() {
        return Ok(Vec::new());
    }
    Ok(klatch::domain::exited_asker_notes(
        &served,
        &t.who(room)?,
        &exited,
        now,
    ))
}

/// What a wait that timed out adds to the marker: helpers hear of askers that are gone (#226).
pub(super) fn print_gone_askers(t: &dyn Transport, a: &WaitArgs, waited: u64) {
    let (me, started) = a.caller;
    let now = started + waited as i64;
    for (room, _) in a.rooms {
        for note in gone_askers_in(t, me, room, now).unwrap_or_default() {
            println!("{note}");
        }
    }
}

/// How often a waiting helper looks for an asker whose process has exited. Short enough to be seconds, long enough
/// that the table is read a dozen times a minute at most.
pub(super) const EXIT_CHECK_EVERY: Duration = Duration::from_secs(5);

/// When the next look for an exited asker is due: at once, then every `EXIT_CHECK_EVERY`.
pub(super) struct EveryFiveSeconds {
    pub(super) next: Instant,
}

impl EveryFiveSeconds {
    fn starting(at: Instant) -> Self {
        EveryFiveSeconds { next: at }
    }

    /// How long until a look is due (zero when it is).
    fn until_due(&self) -> Duration {
        self.next.saturating_duration_since(Instant::now())
    }

    /// True when a look is due, and then the next one is set.
    fn due(&mut self) -> bool {
        let now = Instant::now();
        let due = now >= self.next;
        if due {
            self.next = now + EXIT_CHECK_EVERY;
        }
        due
    }
}

/// End a wait early when an asker the caller serves has exited (#790): its own marker, then one line per asker.
/// True when it printed, so the wait returns.
pub(super) fn print_exited_askers(t: &dyn Transport, a: &WaitArgs, waited: u64) -> bool {
    let (me, started) = a.caller;
    let now = started + waited as i64;
    let notes: Vec<(&str, Vec<String>)> = a
        .rooms
        .iter()
        .map(|(room, _)| {
            (
                room.as_str(),
                exited_askers_in(t, me, room, now).unwrap_or_default(),
            )
        })
        .filter(|(_, notes)| !notes.is_empty())
        .collect();
    if notes.is_empty() {
        return false;
    }
    let rooms: Vec<&str> = notes.iter().map(|(room, _)| *room).collect();
    println!(
        "[klatch wait: asker-exited room={} after={waited}s]",
        rooms.join(",")
    );
    notes
        .iter()
        .flat_map(|(_, n)| n)
        .for_each(|n| println!("{n}"));
    true
}

/// The marker a wait that timed out prints, then the notes that go with it.
pub(super) fn print_timeout(t: &dyn Transport, a: &WaitArgs, timeout: u64) {
    let names: Vec<&str> = a.rooms.iter().map(|(r, _)| r.as_str()).collect();
    println!(
        "[klatch wait: timeout room={} after={timeout}s]",
        names.join(",")
    );
    for note in timeout_notes(t, a, timeout) {
        println!("{note}");
    }
    print_gone_askers(t, a, timeout);
}

/// Before a wait blocks, tell the caller of any cycle through asks it is part of in the rooms it waits on (#273,
/// DESIGN item 90): a member about to wait for an answer that cannot come learns it now, not after the timeout.
/// A room that cannot be read adds nothing.
pub(super) fn print_ask_cycle_notes(t: &dyn Transport, a: &WaitArgs) {
    let (me, now) = a.caller;
    a.rooms
        .iter()
        .filter_map(|(room, _)| Some((t.open_asks(room).ok()?, t.claims(room).ok()?)))
        .flat_map(|(asks, claims)| klatch::domain::cycle_notes(&asks, &claims, me, now))
        .for_each(|note| println!("{note}"));
}

pub(super) fn wait(t: &dyn Transport, a: &WaitArgs) -> Result<()> {
    print_ask_cycle_notes(t, a);
    let (timeout, clamped) = effective_timeout(a.timeout);
    if clamped {
        eprintln!("klatch wait: timeout clamped to {timeout}s (the most one call may wait)");
    }
    let started = Instant::now();
    let deadline = started + Duration::from_secs(timeout);
    let mut exit_check = EveryFiveSeconds::starting(started);
    loop {
        // The token is taken before the poll, so a commit in between is a change the pause sees at once (#616).
        let seen = t.change_token();
        if print_woken(t, a)? {
            return Ok(());
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            print_timeout(t, a, timeout);
            return Ok(());
        }
        if exit_check.due() && print_exited_askers(t, a, started.elapsed().as_secs()) {
            return Ok(());
        }
        // Wake on a store change; the old 200 ms poll stays as the fallback for a store with no token. The pause
        // never outlasts the next look for an exited asker (#790).
        klatch::gateway::pause_for_change(
            &|| t.change_token(),
            seen,
            klatch::gateway::WAKE_FALLBACK
                .min(left)
                .min(exit_check.until_due()),
            klatch::gateway::WAKE_TICK,
        );
    }
}

/// How often `wake` looks. Each look refreshes presence, so this is also the heartbeat of an idle session.
pub(super) const WAKE_POLL: Duration = Duration::from_secs(3);

/// The longest `wake` may watch.
pub(super) const MAX_WAKE_SECONDS: u64 = 3600;

/// The claim to be the member's watcher: a file in the store's directory holding the process id of the newest one.
pub(super) struct WakeLock {
    pub(super) path: std::path::PathBuf,
    pub(super) token: String,
}

impl WakeLock {
    fn is_mine(&self) -> bool {
        std::fs::read_to_string(&self.path).is_ok_and(|t| t == self.token)
    }
}

pub(super) fn wake_lock(me: &str) -> Result<WakeLock> {
    let dir = crate::db_path()?
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let lock = WakeLock {
        path: dir.join(format!("wake-{me}.lock")),
        token: format!("{}-{nanos}", std::process::id()),
    };
    std::fs::write(&lock.path, &lock.token)?;
    Ok(lock)
}

/// Watch the member's rooms for unread messages. The first one ends it with exit 2 and a pointer that carries no
/// message text and no sender (the model reads standard error as the reason it woke, and message text is data); at
/// the timeout it ends quietly with exit 0. It reads nothing: the hook shows the messages as it always does.
pub(super) fn wake(t: &dyn Transport, (me, timeout): (&str, u64)) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(timeout.min(MAX_WAKE_SECONDS));
    // Only the newest watcher of a member stays: each turn starts one, and they must not pile up.
    let lock = wake_lock(me)?;
    loop {
        if !lock.is_mine() {
            return Ok(());
        }
        if let Some((room, unread, newest)) = first_unread_room(t)? {
            // The count and newest seq make the text differ each time: Claude Code drops a repeat (#550).
            // Remember what it pointed at, so the same messages never wake the session again (a watcher
            // started again after every turn would otherwise loop on a message nobody read). They stay
            // unread: the hook, `read` and `wait` still show them.
            // What the view holds back is counted first: marking moves the wake cursor past it.
            let outside = t.outside_view(&room).ok().flatten();
            t.wake_mark(&room)?;
            let mut pointer = klatch::member::wake_pointer(&room, unread, newest);
            if let Some((n, last)) = outside {
                pointer.push(' ');
                pointer.push_str(&klatch::domain::outside_view_line(&room, n, last));
                pointer.push('.');
            }
            eprintln!("{pointer}");
            std::process::exit(2);
        }
        if Instant::now() >= deadline {
            return Ok(());
        }
        std::thread::sleep(WAKE_POLL);
    }
}

/// The first of the member's rooms with unread messages, with its unread count and newest seq.
pub(super) fn first_unread_room(t: &dyn Transport) -> Result<Option<(String, usize, i64)>> {
    for room in t.rooms()? {
        let (unread, newest) = t.wake_summary(&room).unwrap_or((0, 0));
        if unread > 0 {
            return Ok(Some((room, unread, newest)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod wait_tests {
    use super::*;

    #[test]
    fn the_exit_check_is_due_at_once_and_then_not_again_until_its_interval_passes() {
        let mut check = EveryFiveSeconds::starting(Instant::now());
        assert!(check.due(), "the first look is at the start of the wait");
        assert!(!check.due(), "and the next one waits for the interval");
        check.next = Instant::now();
        assert!(check.due(), "once the interval has passed it is due again");
    }

    #[test]
    fn the_timeout_is_capped_at_300_seconds_and_says_when_it_was_clamped() {
        assert_eq!(effective_timeout(60), (60, false));
        assert_eq!(effective_timeout(300), (300, false));
        assert_eq!(effective_timeout(301), (300, true));
        assert_eq!(effective_timeout(u64::MAX), (300, true));
        assert_eq!(effective_timeout(0), (0, false));
    }
}
