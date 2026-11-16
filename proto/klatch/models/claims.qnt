// A model of the claim and lease protocol of one room (DESIGN section 32 item 66, ticket #405).
//
// What it covers, and the code it mirrors:
//   claim      Gateway::claim and SqliteStore::take_claim: overlap by path component, exclusive and shared scopes,
//              the 24 hour cap, expired rows deleted at an attempt, a refusal remembered as a wait, a refusal that
//              closes a cycle of waits told "deadlock" (DESIGN 25.12).
//   release    SqliteStore::release (an expired row that nobody deleted yet can still be released).
//   renew      SqliteStore::renew_claims through touch: renewal to the claim's own ttl (item 52), never shortening,
//              never reviving a lapsed claim.
//   post/park  a post is a sign of life that ends a park and cuts stretched claims back; "parked: ... back by" stretches
//              live claims to the return time, at most seven days out (item 46).
//   settle     the liveness lease rules (item 47, gateway/lease.rs): a live holder is extended, a gone one is cut to
//              the grace period, a stuck one is released, an unreadable one is left alone.
//
// Left out on purpose: ask: claims and their short leases, anchors, release notes, claim ages, rooms (one room is
// enough, claims never conflict across rooms), the busy-for-the-cap verdict (it acts as "stuck" here), members that
// join or leave.
//
// Time unit: one minute. The constants are the real ones in minutes so that a trace replays against the real code
// with a factor of 60 and nothing else (proto/klatch/tests/model_replay.rs).
//
// A model checked result is "model checked up to N steps and M members", never "proven" (RESEARCH-NOTES, Quint).
module claims {
  // ---- constants (real value in seconds / 60) ----
  pure val CAP = 1440            // CLAIM_MAX_SECS, 24 h
  pure val WINDOW = 2            // WAIT_WINDOW_SECS, 120 s
  pure val EXTEND = 10           // DEFAULT_EXTEND_SECS, 600 s
  pure val GRACE = 5             // DEFAULT_GRACE_SECS, 300 s
  pure val DEFAULT_LEASE = 10    // CLAIM_DEFAULT_SECS, 600 s (what an unpark cuts back to)
  pure val PARK_MAX = 10080      // PARKED_MAX_SECS, 7 d
  pure val MAX_TIME = 30000

  // A planted bug, for showing that each invariant can fail. The runner rewrites this one line on a temp copy.
  pure val BUG = "none"

  // ---- the universe the checker explores ----
  pure val MEMBERS = Set("m1", "m2", "m3")
  pure val PATHS = Set("a", "a/b", "c")
  pure val TTLS = Set(2, 10, 1440, 1441)      // 1441 is over the cap and must be refused
  pure val DELTAS = Set(1, 3, 11, 700, 1440, 5000)
  pure val PARK_SPANS = Set(5, 90, 1440, 4320, 20000)   // 20000 is beyond PARK_MAX and is cut to it
  pure val VERDICTS = Set("alive", "gone", "stuck", "unknown")

  // m1 and m2 work in one scope, m3 in another; a shared claim collides with every scope
  pure def scopeOf(m: str): str = if (m == "m3") "s2" else "s1"

  type Claim = { owner: str, path: str, scope: str, expires: int, ttl: int, id: int }
  type Wait = { member: str, path: str, scope: str, ts: int, told: bool }
  type Step = {
    kind: str, member: str, path: str, ttl: int, shared: bool, verdict: str, delta: int, span: int,
    outcome: str, blocker: str, deadlock: bool
  }
  type Snapshot = { now: int, claims: Set[Claim], waits: Set[Wait] }

  var now: int
  var claims: Set[Claim]          // table rows: an expired row stays until some claim attempt deletes it
  var waits: Set[Wait]
  var parkedUntil: str -> int     // 0 when not parked; a park row stays until a sign of life
  var everParked: Set[str]        // ghost: members that parked at some time
  var nextId: int
  var last: Step                  // the action just taken, for replay and for the step invariants
  var prev: Snapshot              // the state before it

  pure val NO_STEP: Step = {
    kind: "init", member: "", path: "", ttl: 0, shared: false, verdict: "", delta: 0, span: 0,
    outcome: "", blocker: "", deadlock: false
  }
  pure val NO_CLAIM: Claim = { owner: "", path: "", scope: "", expires: 0, ttl: 0, id: -1 }

  // ---- the path and scope relations (domain/text.rs overlaps, domain/scope.rs scopes_collide) ----
  pure def overlap(x: str, y: str): bool =
    x == y or (x == "a" and y == "a/b") or (x == "a/b" and y == "a")
  pure def overlapBug(x: str, y: str): bool =
    if (BUG == "equalOnly") x == y else overlap(x, y)
  pure def collide(x: str, y: str): bool = x == y or x == "*" or y == "*"
  pure def collideBug(x: str, y: str): bool =
    if (BUG == "sharedIgnored") x == y else collide(x, y)

  val liveClaims: Set[Claim] = claims.filter(c => c.expires > now)

  // ---- who waits for whom (gateway: closes_cycle, reaches, blockers) ----
  pure def succs(x: str, ws: Set[Wait], held: Set[Claim]): Set[str] =
    ws.filter(w => w.member == x).fold(Set(), (acc, w) =>
      acc.union(held.filter(h => h.owner != w.member and overlap(h.path, w.path) and collide(h.scope, w.scope))
        .map(h => h.owner)))

  pure def reaches(start: str, target: str, ws: Set[Wait], held: Set[Claim]): bool = {
    pure def grow(r) = r.union(r.fold(Set(), (a, x) => a.union(succs(x, ws, held))))
    MEMBERS.fold(Set(start), (acc, _) => grow(acc)).contains(target)
  }

  // The same question asked another way, for the invariants: whether x waits for something y holds, and walks of at
  // most three of those steps (three members, so a cycle is at most three), written with quantifiers over the members
  // instead of the frontier sets the action uses. Apalache copes with this form far better than with a closure built
  // as a set of pairs.
  pure def adjacent(x: str, y: str, ws: Set[Wait], cs: Set[Claim], t: int, win: int): bool =
    ws.exists(w => w.member == x and w.ts >= t - win and
      cs.exists(h => h.expires > t and h.owner == y and h.owner != w.member
        and overlap(h.path, w.path) and collide(h.scope, w.scope)))
  pure def walk2(x: str, y: str, ws: Set[Wait], cs: Set[Claim], t: int, win: int): bool =
    adjacent(x, y, ws, cs, t, win)
      or MEMBERS.exists(z => adjacent(x, z, ws, cs, t, win) and adjacent(z, y, ws, cs, t, win))
  pure def walk3(x: str, y: str, ws: Set[Wait], cs: Set[Claim], t: int, win: int): bool =
    walk2(x, y, ws, cs, t, win) or MEMBERS.exists(z => walk2(x, z, ws, cs, t, win) and adjacent(z, y, ws, cs, t, win))

  // The first row in table order (insertion order) is the one a refusal names.
  pure def oldest(cs: Set[Claim]): Claim =
    cs.fold(NO_CLAIM, (a, c) => if (a.id < 0 or c.id < a.id) c else a)

  // ---- bookkeeping shared by every action ----
  action record(s: Step): bool = all {
    last' = s,
    prev' = { now: now, claims: claims, waits: waits },
  }

  // ---- actions ----
  action tick(d: int): bool = all {
    now + d <= MAX_TIME,
    now' = now + d,
    claims' = claims, waits' = waits, parkedUntil' = parkedUntil, everParked' = everParked, nextId' = nextId,
    record({ ...NO_STEP, kind: "tick", delta: d }),
  }

  // A claim attempt. Over the cap it is refused without touching anything.
  action claim(m: str, p: str, ttl: int, shared: bool): bool = {
    val sc = if (shared) "*" else scopeOf(m)
    val tooLong = if (BUG == "capOff") false else ttl > CAP
    val s0 = { ...NO_STEP, kind: "claim", member: m, path: p, ttl: ttl, shared: shared }
    if (tooLong) all {
      claims' = claims, waits' = waits, now' = now, parkedUntil' = parkedUntil, everParked' = everParked,
      nextId' = nextId,
      record({ ...s0, outcome: "invalid" }),
    } else {
      // the attempt first deletes expired rows and waits older than the window
      val held = if (BUG == "expiredBlocks") claims else liveClaims
      val ws = if (BUG == "windowIgnored") waits else waits.filter(w => w.ts >= now - WINDOW)
      val blockers = held.filter(h =>
        h.owner != m and overlapBug(h.path, p) and collideBug(h.scope, sc))
      if (blockers.size() > 0) {
        val b = oldest(blockers)
        val others = ws.filter(w => not(w.member == m and w.path == p))
        val mine0 = { member: m, path: p, scope: sc, ts: now, told: false }
        val closes = reaches(b.owner, m, others.union(Set(mine0)), held)
        val dl = if (BUG == "deadlockNever") false else closes
        val others2 = if (BUG == "tellAll" and dl) others.map(w => { ...w, told: true }) else others
        all {
          claims' = held, waits' = others2.union(Set({ ...mine0, told: dl })),
          now' = now, parkedUntil' = parkedUntil, everParked' = everParked, nextId' = nextId,
          record({ ...s0, outcome: "conflict", blocker: b.owner, deadlock: dl }),
        }
      } else {
        // taking the path ends this member's waits for it and for anything it overlaps
        val ws2 = ws.filter(w => not(w.member == m and overlap(w.path, p)))
        val fresh = { owner: m, path: p, scope: sc, expires: now + ttl, ttl: ttl, id: nextId }
        all {
          claims' = held.filter(c => not(c.owner == m and c.path == p)).union(Set(fresh)),
          waits' = ws2, nextId' = nextId + 1,
          now' = now, parkedUntil' = parkedUntil, everParked' = everParked,
          record({ ...s0, outcome: "ok" }),
        }
      }
    }
  }

  action release(m: str, p: str): bool = {
    val mine = claims.filter(c => if (BUG == "releaseAny") c.path == p else c.owner == m and c.path == p)
    all {
      claims' = claims.exclude(mine),
      waits' = waits, now' = now, parkedUntil' = parkedUntil, everParked' = everParked, nextId' = nextId,
      record({ ...NO_STEP, kind: "release", member: m, path: p,
               outcome: if (mine.size() > 0) "ok" else "notfound" }),
    }
  }

  // Renewal (item 52), the effect of any activity in the room: each live claim goes to now plus its own ttl, only
  // when that is later, and an expired row is not revived.
  pure def renewed(cs: Set[Claim], m: str, t: int): Set[Claim] =
    cs.map(c => {
      val due = c.owner == m and c.ttl > 0
        and (if (BUG == "renewShortens") true else c.expires < t + c.ttl)
        and (if (BUG == "renewRevives") true else c.expires > t)
      if (due) { ...c, expires: t + c.ttl } else c
    })

  action activity(m: str): bool = all {
    claims' = renewed(claims, m, now),
    waits' = waits, now' = now, parkedUntil' = parkedUntil, everParked' = everParked, nextId' = nextId,
    record({ ...NO_STEP, kind: "renew", member: m }),
  }

  // A post that is not a park: renewal (the post touches the room), then the sign of life ends a park and cuts
  // whatever it stretched back to a normal lease.
  action post(m: str): bool = {
    val renewedClaims = renewed(claims, m, now)
    val wasParked = parkedUntil.get(m) != 0
    // never below the claim's own ttl from now (#649): the renewal just ran, and a cut under it would leave a long
    // claim short until the member's next action
    def cutTo(c: Claim): int =
      now + (if (BUG == "cutBelowTtl" or c.ttl < DEFAULT_LEASE) DEFAULT_LEASE else c.ttl)
    all {
      claims' = if (wasParked) renewedClaims.map(c =>
                  if (c.owner == m and c.expires > cutTo(c)) { ...c, expires: cutTo(c) } else c)
                else renewedClaims,
      parkedUntil' = parkedUntil.set(m, 0),
      waits' = waits, now' = now, everParked' = everParked, nextId' = nextId,
      record({ ...NO_STEP, kind: "post", member: m }),
    }
  }

  // "parked: ..., back by <span>": the post renews first, then the live claims are stretched to the return time.
  action park(m: str, spanMins: int): bool = {
    val capped = if (BUG == "parkUncapped") spanMins else (if (spanMins > PARK_MAX) PARK_MAX else spanMins)
    val back = now + capped
    val renewedClaims = renewed(claims, m, now)
    val held = parkedUntil.get(m)    // what an earlier park still holds; 0 when not parked
    // A claim short of the return time is stretched to it. A claim beyond it (or its own ttl from now, if later) but
    // within what the earlier park held is that park's stretch and is cut back to it (#647); one beyond the earlier
    // return time was never stretched by it and stays.
    def parked(c: Claim): int = {
      val floor = if (back > now + c.ttl) back else now + c.ttl
      if (BUG == "parkShortens") (if (c.expires > back) back else c.expires)
      else if (c.expires < back) back
      else if (BUG != "reparkKeeps" and held > now and c.expires <= held and c.expires > floor) floor
      else c.expires
    }
    all {
      claims' = renewedClaims.map(c =>
        if (c.owner == m and (BUG == "parkRevives" or c.expires > now)) { ...c, expires: parked(c) } else c),
      parkedUntil' = parkedUntil.set(m, back),
      everParked' = everParked.union(Set(m)),
      waits' = waits, now' = now, nextId' = nextId,
      record({ ...NO_STEP, kind: "park", member: m, span: spanMins }),
    }
  }

  // Someone other than `owner` asks about claims, so the lease rules look at `owner` (gateway/lease.rs).
  action settle(asker: str, owner: str, v: str): bool = {
    val mineRows = claims.filter(c => c.owner == owner)
    val extendTo = if (BUG == "extendOverCap") now + CAP + EXTEND else now + EXTEND
    val after =
      if (v == "alive") claims.map(c =>
        if (c.owner == owner and c.expires < extendTo) { ...c, expires: extendTo } else c)
      else if (v == "gone") claims.map(c =>
        if (c.owner == owner and c.expires > now + GRACE) { ...c, expires: now + GRACE } else c)
      else if (v == "stuck") (if (BUG == "stuckKeeps") claims else claims.exclude(mineRows))
      else claims
    all {
      asker != owner,
      claims' = after,
      waits' = waits, now' = now, parkedUntil' = parkedUntil, everParked' = everParked, nextId' = nextId,
      record({ ...NO_STEP, kind: "settle", member: asker, blocker: owner, verdict: v }),
    }
  }

  action init: bool = all {
    now' = 0,
    claims' = Set(),
    waits' = Set(),
    parkedUntil' = MEMBERS.mapBy(m => 0),
    everParked' = Set(),
    nextId' = 0,
    last' = NO_STEP,
    prev' = { now: 0, claims: Set(), waits: Set() },
  }

  action step: bool = {
    nondet m = MEMBERS.oneOf()
    nondet o = MEMBERS.oneOf()
    nondet p = PATHS.oneOf()
    nondet ttl = TTLS.oneOf()
    nondet shared = Set(false, true).oneOf()
    nondet d = DELTAS.oneOf()
    nondet sp = PARK_SPANS.oneOf()
    nondet v = VERDICTS.oneOf()
    any {
      tick(d),
      claim(m, p, ttl, shared),
      release(m, p),
      activity(m),
      post(m),
      park(m, sp),
      settle(m, o, v),
    }
  }

  // Only claims, releases and time: what the deadlock invariants need. Apalache is much faster on it than on `step`,
  // which matters because the cycle test is the expensive part of the model.
  action stepCore: bool = {
    nondet m = MEMBERS.oneOf()
    nondet p = PATHS.oneOf()
    nondet ttl = Set(10, 1441).oneOf()
    nondet shared = Set(false, true).oneOf()
    nondet d = Set(1, 3).oneOf()
    any {
      tick(d),
      claim(m, p, ttl, shared),
      release(m, p),
    }
  }

  // =====================================================================================================
  // Invariants. Each has a name that the runner, CONTRIBUTING.md and RESEARCH-NOTES use.
  // =====================================================================================================

  // No two live claims by different members overlap in colliding scopes (exclusive and shared scopes).
  val NoOverlappingLiveExclusive: bool =
    liveClaims.forall(x => liveClaims.forall(y =>
      x.owner == y.owner or not(overlap(x.path, y.path) and collide(x.scope, y.scope))))

  // A lease never exceeds the 24 hour cap, except for a member that has parked (item 46 stretches to a return time).
  val LeaseWithinCap: bool =
    liveClaims.forall(c => everParked.contains(c.owner) or c.expires - now <= CAP)

  // Even a park never stretches a lease past the park maximum.
  val LeaseWithinParkMax: bool =
    liveClaims.forall(c => c.expires - now <= PARK_MAX)

  // Settling a holder judged stuck leaves it holding nothing.
  val StuckOwnerHoldsNothing: bool =
    (last.kind == "settle" and last.verdict == "stuck") implies
      claims.filter(c => c.owner == last.blocker).size() == 0

  // A release only ever removes the releasing member's own claim.
  val ReleaseOnlyOwn: bool =
    (last.kind == "release") implies
      prev.claims.exclude(claims).forall(c => c.owner == last.member and c.path == last.path)

  // A refusal is always by a live claim, never by an expired row nobody has deleted yet.
  val ExpiredNeverBlocks: bool =
    (last.kind == "claim" and last.outcome == "conflict") implies
      prev.claims.exists(c => c.owner == last.blocker and c.expires > prev.now)

  // A row that had lapsed is not live again after the step unless the step is a liveness extension of an alive owner.
  val OnlyLivenessRevives: bool =
    prev.claims.forall(p => claims.forall(c =>
      c.id != p.id or not(p.expires <= prev.now and c.expires > now)
        or (last.kind == "settle" and last.verdict == "alive")))

  // Renewal (activity, a post, a park's own renewal) never revives a lapsed claim and never shortens a lease.
  val RenewNeverRevives: bool =
    (last.kind == "renew" or last.kind == "post" or last.kind == "park") implies
      prev.claims.forall(p => claims.forall(c =>
        c.id != p.id or not(p.expires <= prev.now and c.expires > p.expires)))

  val RenewNeverShortens: bool =
    (last.kind == "renew") implies
      prev.claims.forall(p => claims.forall(c => c.id != p.id or c.expires >= p.expires))

  // A lease only shrinks by a lease rule: a liveness rule (settle), the cut back that ends a park (post), or a shorter
  // re-park cutting the earlier stretch, never below the new return time or the claim's own ttl from now (#647).
  val ExpiryShrinksOnlyByLeaseRules: bool =
    prev.claims.forall(p => claims.forall(c =>
      c.id != p.id or c.expires >= p.expires or last.kind == "settle" or last.kind == "post"
        or (last.kind == "park" and c.expires >= now + c.ttl and c.expires >= parkedUntil.get(c.owner))))

  // The deadlock flag is exactly "the refused attempt closes a cycle of waits", checked with a second algorithm.
  val DeadlockFlagExact: bool =
    (last.kind == "claim" and last.outcome == "conflict") implies
      (last.deadlock == walk3(last.blocker, last.member, waits, claims, now, WINDOW))

  // A refusal tells only the requester: no other member's wait changes at that step, so when an attempt closes a cycle
  // the closer yields and nobody else is newly told. (A told wait from an earlier step can still be on the cycle, if
  // its member did not yield; that is the member's choice, not a second yielder made by this step.)
  val RefusalTellsOnlyTheRequester: bool =
    (last.kind == "claim" and last.outcome == "conflict") implies
      waits.forall(w => w.member == last.member or prev.waits.contains(w))

  // Activity renews every live claim of the member to at least now plus its own ttl (item 52). Held after #649: the
  // cut back that ends a park never goes below a claim's own ttl. (Before the fix it failed after a post by a member
  // that had a park row: the cut to 10 minutes ran after the renewal.)
  val ActivityRenewsToOwnTtl: bool =
    (last.kind == "post" or last.kind == "renew") implies
      liveClaims.forall(c => c.owner != last.member or c.ttl == 0 or c.expires >= now + c.ttl)

  // A parked member's claims end no later than its stated return time (or the cap). Held after #647: a shorter re-park
  // cuts the earlier, longer stretch. (Before the fix it failed: a park never shortened, so the old stretch stayed.)
  val ParkedLeaseEndsAtReturn: bool =
    liveClaims.forall(c =>
      parkedUntil.get(c.owner) == 0 or c.expires - now <= CAP or c.expires <= parkedUntil.get(c.owner))

  // Every invariant that must hold, for a quick run; the runner also checks each alone so a failure names itself.
  val AllInvariants: bool = and {
    NoOverlappingLiveExclusive, LeaseWithinCap, LeaseWithinParkMax, StuckOwnerHoldsNothing, ReleaseOnlyOwn,
    ExpiredNeverBlocks, OnlyLivenessRevives, RenewNeverRevives, RenewNeverShortens,
    ExpiryShrinksOnlyByLeaseRules, DeadlockFlagExact, RefusalTellsOnlyTheRequester, ActivityRenewsToOwnTtl,
    ParkedLeaseEndsAtReturn,
  }

  // ---- Known findings: these are expected to FAIL, and each has a ticket (RESEARCH-NOTES, Quint) ----

  // F1. Strict 24 hour cap including a parked member: fails, a park stretches a claim to seven days (item 46 vs D5).
  val StrictLeaseCap: bool = liveClaims.forall(c => c.expires - now <= CAP)

  // F3. A cycle of live waits always has a member told to yield: fails, a cycle can be completed by a claim being
  // taken (a hold edge appears), not by a refusal, and nobody is told until one of them retries.
  val EveryCycleHasATold: bool =
    MEMBERS.forall(x => not(walk3(x, x, waits, claims, now, WINDOW)) or
      waits.exists(w => w.ts >= now - WINDOW and w.told
        and walk3(x, w.member, waits, claims, now, WINDOW) and walk3(w.member, x, waits, claims, now, WINDOW)))

  // ---- witnesses: each is "never happens" and is made to fail on purpose, to give a short trace of that event ----
  val W_deadlock: bool = not(last.kind == "claim" and last.deadlock)
  val W_conflict: bool = not(last.kind == "claim" and last.outcome == "conflict")
  val W_invalid: bool = not(last.kind == "claim" and last.outcome == "invalid")
  val W_renewMoves: bool =
    not(last.kind == "renew" and prev.claims.exists(p => claims.exists(c => c.id == p.id and c.expires > p.expires)))
  val W_parkStretch: bool =
    not(last.kind == "park" and prev.claims.exists(p => claims.exists(c => c.id == p.id and c.expires > p.expires + CAP)))
  val W_revive: bool =
    not(last.kind == "settle" and prev.claims.exists(p => claims.exists(c =>
      c.id == p.id and p.expires <= prev.now and c.expires > now)))
  val W_cutBack: bool =
    not(last.kind == "post" and prev.claims.exists(p => claims.exists(c => c.id == p.id and c.expires < p.expires)))
  // a cut back that lands above the default lease: the claim's own ttl is the floor (#649)
  val W_cutKeepsTtl: bool =
    not(last.kind == "post" and prev.claims.exists(p => claims.exists(c =>
      c.id == p.id and c.expires < p.expires and c.expires > now + DEFAULT_LEASE)))
  // a re-park that shortens a claim: the earlier stretch cut to the new return time (#647)
  val W_reparkCuts: bool =
    not(last.kind == "park" and prev.claims.exists(p => claims.exists(c => c.id == p.id and c.expires < p.expires)))
  val W_stuck: bool = not(last.kind == "settle" and last.verdict == "stuck" and prev.claims.size() > claims.size())
  val W_gone: bool =
    not(last.kind == "settle" and last.verdict == "gone"
      and prev.claims.exists(p => claims.exists(c => c.id == p.id and c.expires < p.expires)))
  val W_shared: bool = not(last.kind == "claim" and last.outcome == "conflict" and last.shared)
  val W_postKeeps: bool =
    not(last.kind == "post" and liveClaims.exists(c => c.owner == last.member and c.expires - now > DEFAULT_LEASE))
  val W_staleWait: bool =
    not(last.kind == "claim" and last.outcome == "conflict" and not(last.deadlock)
      and walk3(last.blocker, last.member, waits.union(prev.waits), claims, now, 100000))
  val W_releaseNotFound: bool = not(last.kind == "release" and last.outcome == "notfound")
}
