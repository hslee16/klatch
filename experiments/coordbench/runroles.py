"""The role members of the arms K3 and B1-roles (split out of run.py, #1455): the grants in the run store, the replacements the
supervisor asks for, and the roles record. A mixin of runexec.RunExecution: it reads the run's state from self."""
import os
import subprocess

import roles as roles_mod
from runenv import append_exclude


class RoleSupport:
    def grant_roles(self, env):
        """The K3 role grants, in the run store (roles.py explains the seam): the integrator role by `klatch admin integrator`
        at a pseudo-terminal, a clearance when the scenario sets one. klatch has no supervisor role, so the supervisor is an
        ordinary member. A grant that fails is recorded and noted, never an invalid run."""
        seam = roles_mod.AdminSeam(self.a.klatch_bin, env, self.scratch)
        if "integrator" in self.rcfg["members"]:
            self.grant_integrator(seam)
        for member, label in sorted((self.rcfg.get("clearance") or {}).items()):
            if member in self.rcfg["members"]:
                self.grant_clearance(seam, member, label)

    def grant_integrator(self, seam):
        ok, why = roles_mod.grant_integrator(seam, "integrator")
        self.role_info["integrator"].update(granted=bool(ok), grant_error=None if ok else why)
        if not ok:
            self.notes.append("role integrator was not granted: %s" % why)

    def grant_clearance(self, seam, member, label):
        ok, why = roles_mod.set_clearance(seam, member, label)
        self.role_info[member]["clearance"] = label if ok else None
        if not ok:
            self.notes.append("clearance of %s was not set: %s" % (member, why))

    def role_entry(self, key):
        s = self.allsums[key]
        rc = self.rcs.get(key)
        state = self.stop_reason.get(key) or ("ok" if rc == 0 else "failed")
        return {"turns": s["turns"], "tokens": s["tokens"], "cost_usd": round(s["cost_usd"], 6),
                "cost_estimated": s["cost_estimated"], "tool_calls": s["tool_calls"]["total"], "messages": s["messages"],
                "wall_s": round(self.ended_at.get(key, self.stopped_t) - self.started_at[key], 3), "exit": rc, "state": state,
                "models": s["models"]}

    def role_record(self):
        """rec["roles"]: per role member (and per replacement the supervisor started) its turns, tokens, cost, wall time,
        exit and state, apart from the workers' numbers; spend.usd stays the team total and gains workers_usd and roles_usd."""
        rec, rcfg = self.rec, self.rcfg
        members, repl = {}, {}
        for k in sorted(self.role_keys):
            members[k] = dict(self.role_entry(k), **self.role_info.get(k, {}))
            if members[k]["state"] != "ok":
                self.notes.append("role %s: %s (exit %s)" % (k, members[k]["state"], members[k]["exit"]))
        for k in sorted(self.repl_keys):
            repl[k] = dict(self.role_entry(k), replaces=[x["member"] for x in self.restarts if x["replacement"] == k][0])
        rec["roles"] = {"model": rcfg["model"], "per_role_usd": rcfg["per_role_usd"], "grace_s": rcfg["grace_s"],
                        "max_restarts": rcfg["max_restarts"], "members": members, "replacements": repl,
                        "restarts": self.restarts}
        roles_usd = sum(m["cost_usd"] for m in list(members.values()) + list(repl.values()))
        rec["spend"]["roles_usd"] = round(roles_usd, 6)
        rec["spend"]["workers_usd"] = round(rec["spend"]["usd"] - roles_usd, 6)
        rec["models"]["roles"] = rcfg["model"]

    def restart_refusal(self, member):
        """Why the supervisor's request to replace member is refused, or None when it may be answered."""
        done = [x for x in self.restarts if x["member"] == member and x["started"]]
        if member not in self.team:
            return "%s is not a worker" % member
        if len(done) >= self.rcfg["max_restarts"]:
            return "restart limit reached (%d for %s)" % (self.rcfg["max_restarts"], member)
        if self.procs[member].poll() is None:
            return "%s is still running" % member
        return None

    def start_replacement(self, member, name2):
        clone = self.clone_path(name2)
        self.git(["clone", "-q", "--no-local", self.origin, clone], self.scratch, self.henv)  # #1435: see checks_now
        append_exclude(clone)
        self.clones[name2] = clone
        if self.arm.startswith("K") and self.arm != "K1":
            p = subprocess.run([self.a.klatch_bin, "init", "--scope", "project", "--yes", "--as-member", name2],
                               cwd=clone, env=dict(self.henv, **self.kenv, KLATCH_NAME=name2), stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True)
            if p.returncode != 0:
                raise RuntimeError("klatch init failed: %s" % p.stdout.strip()[-120:])
        note = ("You are `%s`, a replacement for `%s`, whose session stopped before it finished its job. Its job "
                "is the text above. Whatever it published is in the shared repository and in your team's shared "
                "channel; its unpublished work is gone. Finish the job." % (name2, member))
        self.start(name2, name2, 1, self.task_of[member] + "\n\n" + note, {"replacement": True})

    def answer_restart(self, member):
        """Decide one restart request: (the replacement's name or None, the refusal reason or None)."""
        reason = self.restart_refusal(member)
        name2 = "%s-%d" % (member, len([x for x in self.restarts if x["member"] == member and x["started"]]) + 2)
        if reason is None:
            try:
                self.start_replacement(member, name2)
            except (RuntimeError, OSError) as e:
                reason = "could not start %s: %s" % (name2, e)
        return (name2 if reason is None else None), reason

    def handle_restarts(self):
        """Answer the supervisor's restart requests (restart_member.py): a replacement is a new member with a clone of its
        own, started only for a worker whose process has ended, within the limit; the request file is consumed."""
        for fn in sorted(os.listdir(self.restart_dir)):
            if not fn.endswith(".request"):
                continue
            member = fn[:-len(".request")]
            os.remove(os.path.join(self.restart_dir, fn))
            name2, reason = self.answer_restart(member)
            if reason is None:
                self.restarts.append({"member": member, "replacement": name2, "started": True, "reason": None})
                answer = "started: %s" % name2
            else:
                self.restarts.append({"member": member, "replacement": None, "started": False, "reason": reason})
                answer = "refused: %s" % reason
            with open(os.path.join(self.restart_dir, member + ".result"), "w") as f:
                f.write(answer + "\n")
