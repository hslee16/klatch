"""What the run record says about what ran (#1530, #1547): metric_version from the start, the prompts of role members, a hash of
the arm texts, and the klatch binary's own identity. No model, no network."""
import argparse
import os
import re
import shutil
import stat
import tempfile
import unittest
from unittest import mock

from helpers import BENCH
import fixture as fixture_mod
import metricver
import run
import runenv
import runidentity
import schema
import test_run_fake

HEX64 = "^[0-9a-f]{64}$"
VERSION_LINE = "klatch 0.9.9 (protocol 3, store schema 41)"


def fake_binary(directory, line=VERSION_LINE):
    path = os.path.join(directory, "klatch")
    with open(path, "w") as f:
        f.write("#!/bin/sh\necho '%s'\n" % line)
    os.chmod(path, stat.S_IRWXU)
    return path


class ArmsCopy(unittest.TestCase):
    """A scratch copy of the arm texts, so a test can change one."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="cb-ident-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.arms = os.path.join(self.tmp, "arms")
        shutil.copytree(os.path.join(BENCH, "arms"), self.arms)
        self.args = argparse.Namespace(arms_dir=self.arms, wall_s=1800, max_turns=60, roles=None, role_usd=None,
                                       roles_grace_s=None, roles_model=None, model=None, max_run_usd=3.0,
                                       scenarios_dir=os.path.join(BENCH, "scenarios"))
        self.fx = fixture_mod.Fixture("trivial", fixtures_dir=os.path.join(BENCH, "fixtures"), checks_dir=os.path.join(BENCH, "checks"))

    def edit(self, name, extra="\nchanged\n"):
        with open(os.path.join(self.arms, name), "a") as f:
            f.write(extra)

    def prompts(self, arm):
        return runidentity.prompt_hashes(self.args, self.fx, arm)


class RolePrompts(ArmsCopy):
    def test_role_arms_hash_the_role_member_prompts_too(self):
        for arm in ("K3", "B1-roles"):
            h = self.prompts(arm)
            self.assertEqual(sorted(h), ["a", "b", "integrator", "supervisor"], arm)
            for v in h.values():
                self.assertRegex(v, HEX64)

    def test_arms_without_roles_hash_workers_only(self):
        self.assertEqual(sorted(self.prompts("K2")), ["a", "b"])

    def test_a_role_prompt_change_changes_that_role_hash_only(self):
        before = self.prompts("K3")
        with open(os.path.join(self.arms, "K3.md")) as f:
            self.assertIn("supervisor", f.read().lower())
        import roles
        secs = roles.sections_of(self.arms, "K3")
        path = os.path.join(self.arms, "K3.md")
        with open(path) as f:
            text = f.read()
        body = secs["supervisor"]
        with open(path, "w") as f:
            f.write(text.replace(body, body + "\nan extra sentence for the supervisor\n"))
        after = self.prompts("K3")
        self.assertNotEqual(after["supervisor"], before["supervisor"])
        self.assertEqual(after["integrator"], before["integrator"])

    def test_an_unrelated_file_changes_nothing(self):
        before = self.prompts("K3")
        self.edit("PILOT.md")
        self.edit("REVIEW.md")
        self.assertEqual(self.prompts("K3"), before)


class ArmTextHash(ArmsCopy):
    def test_the_hash_changes_when_an_arm_text_the_arm_uses_changes(self):
        h = runidentity.arm_text_sha256(self.arms, "K2", "T-clone")
        self.assertRegex(h, HEX64)
        self.edit("K2.md")
        self.assertNotEqual(runidentity.arm_text_sha256(self.arms, "K2", "T-clone"), h)

    def test_the_common_text_is_part_of_it(self):
        h = runidentity.arm_text_sha256(self.arms, "B0", "T-clone")
        self.edit("common.md")
        self.assertNotEqual(runidentity.arm_text_sha256(self.arms, "B0", "T-clone"), h)

    def test_a_role_arm_covers_its_base_arm_and_its_own_file(self):
        h = runidentity.arm_text_sha256(self.arms, "K3", "T-clone")
        self.edit("K2.md")
        h2 = runidentity.arm_text_sha256(self.arms, "K3", "T-clone")
        self.assertNotEqual(h2, h)
        self.edit("K3.md")
        self.assertNotEqual(runidentity.arm_text_sha256(self.arms, "K3", "T-clone"), h2)

    def test_another_arm_or_an_unrelated_file_changes_nothing(self):
        h = runidentity.arm_text_sha256(self.arms, "K2", "T-clone")
        self.edit("K1.md")
        self.edit("PILOT.md")
        self.assertEqual(runidentity.arm_text_sha256(self.arms, "K2", "T-clone"), h)

    def test_the_shared_tree_variant_reads_the_shared_files(self):
        h = runidentity.arm_text_sha256(self.arms, "K2", "T-shared")
        self.edit("K2-shared.md")
        self.assertNotEqual(runidentity.arm_text_sha256(self.arms, "K2", "T-shared"), h)

    def test_a_missing_text_gives_none(self):
        self.assertIsNone(runidentity.arm_text_sha256(os.path.join(self.tmp, "nowhere"), "K2", "T-clone"))


class KlatchBinary(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="cb-kbin-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)

    def test_the_binary_identity_comes_from_the_binary_itself(self):
        path = fake_binary(self.tmp)
        sha, schema_no, version = runenv.klatch_facts(path, dict(os.environ))
        self.assertRegex(sha, HEX64)
        self.assertEqual(schema_no, 41)
        self.assertEqual(version, VERSION_LINE)

    def test_no_binary_gives_nothing(self):
        self.assertEqual(runenv.klatch_facts(None, {}), (None, None, None))

    def test_a_binary_that_fails_keeps_its_hash(self):
        path = os.path.join(self.tmp, "klatch")
        with open(path, "w") as f:
            f.write("#!/bin/sh\nexit 3\n")
        os.chmod(path, stat.S_IRWXU)
        sha, schema_no, version = runenv.klatch_facts(path, dict(os.environ))
        self.assertRegex(sha, HEX64)
        self.assertIsNone(schema_no)
        self.assertIsNone(version)


class RecordFromARun(test_run_fake.Base):
    def record(self, **kw):
        code, _, _ = self.go(self.argv("B0", runs=1, **kw))
        self.assertEqual(code, 0)
        return test_run_fake.read_json(os.path.join(self.results, "t1", "trivial", "B0", "t1-trivial-b0-r01.json"))

    def test_a_run_that_crashes_before_metrics_has_metric_version(self):
        with mock.patch.object(run.Suite, "execute", side_effect=RuntimeError("boom")):
            code, _, _ = self.go(self.argv("B0", runs=1))
        rec = test_run_fake.read_json(os.path.join(self.results, "t1", "trivial", "B0", "t1-trivial-b0-r01.json"))
        self.assertEqual(rec["outcome"], "error")
        self.assertEqual(rec["metric_version"], metricver.METRIC_VERSION)
        self.assertNotIn("coordination_context", rec)  # it never reached the metrics

    def test_the_record_carries_the_arm_text_hash_and_keeps_the_label(self):
        rec = self.record(arm_text_version="draft-9")
        self.assertEqual(rec["arm_text_version"], "draft-9")
        self.assertRegex(rec["arm_text_sha256"], HEX64)

    def test_the_klatch_block_names_the_binary_and_the_bench_commit_separately(self):
        binary = fake_binary(self.tmp)
        with mock.patch.object(test_run_fake, "KLATCH_BIN", binary):
            rec = self.record()
        k = rec["klatch"]
        self.assertEqual(k["version_output"], VERSION_LINE)
        self.assertEqual(k["store_schema"], 41)
        self.assertRegex(k["binary_sha256"], HEX64)
        self.assertRegex(k["bench_commit"], "^[0-9a-f]{40}$")
        self.assertIsNone(k["commit"])  # nothing in the binary's output says its commit: not guessed from the bench repository

    def test_a_klatch_commit_given_on_the_command_line_is_the_commit(self):
        rec = self.record(klatch_commit="c0ffee" * 6 + "c0ff")
        self.assertEqual(rec["klatch"]["commit"], "c0ffee" * 6 + "c0ff")


class OldRecords(unittest.TestCase):
    def old(self):
        from helpers import make_record
        return make_record()

    def test_an_old_record_without_the_new_fields_validates(self):
        self.assertEqual(schema.validate_record(self.old()), [])

    def test_new_fields_are_checked_when_present(self):
        r = self.old()
        r["arm_text_sha256"] = "x"
        r["klatch"]["version_output"] = 3
        r["prompt_sha256"] = ["no"]
        errs = " ".join(schema.validate_record(r))
        self.assertIn("arm_text_sha256", errs)
        self.assertIn("klatch.version_output", errs)
        self.assertIn("prompt_sha256", errs)

    def test_new_fields_of_the_right_shape_pass(self):
        r = self.old()
        r["arm_text_sha256"] = "a" * 64
        r["klatch"].update({"version_output": VERSION_LINE, "bench_commit": "b" * 40})
        r["prompt_sha256"] = {"a": "c" * 64}
        self.assertEqual(schema.validate_record(r), [])

    def test_the_report_reads_old_and_new_records_together(self):
        import report
        new = self.old()
        new["arm_text_sha256"] = "a" * 64
        new["klatch"]["version_output"] = VERSION_LINE
        c = report.conditions([self.old(), new])
        self.assertEqual(c["arm text sha256"], ["a" * 64])
        self.assertEqual(c["klatch version"], [VERSION_LINE])


if __name__ == "__main__":
    unittest.main()
