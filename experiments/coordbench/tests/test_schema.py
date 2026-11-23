import copy
import unittest

from helpers import BENCH, make_record  # noqa: F401
import schema


class Schema(unittest.TestCase):
    def test_a_complete_record_is_valid(self):
        self.assertEqual(schema.validate_record(make_record()), [])

    def test_every_spec_key_is_required_and_null_is_allowed_not_omission(self):
        for key in schema.REQUIRED_KEYS:
            r = make_record()
            del r[key]
            self.assertTrue(schema.validate_record(r), key)
        r = make_record()
        r["time_to_first_pass_s"] = None
        r["claude_code_version"] = None
        self.assertEqual(schema.validate_record(r), [])

    def test_the_spec_lists_these_keys(self):
        want = {"schema", "run_id", "milestone", "scenario", "topology", "arm", "arm_text_version", "fixture", "seed",
                "valid", "supersedes", "models", "claude_code_version", "klatch", "temperature", "caps", "start", "end",
                "wall_s", "time_to_first_pass_s", "outcome", "spend", "turns", "tokens", "tool_calls", "metrics",
                "outcome_checks", "artifacts", "notes"}
        self.assertEqual(set(schema.REQUIRED_KEYS), want)

    def test_outcome_must_be_one_of_the_five(self):
        for good in ("ok", "timeout", "spend-cap", "crash", "error"):
            r = make_record(outcome=good)
            self.assertEqual(schema.validate_record(r), [], good)
        self.assertTrue(schema.validate_record(make_record(outcome="great")))

    def test_nested_keys_are_checked(self):
        r = make_record()
        del r["metrics"]["messages"]
        self.assertTrue(schema.validate_record(r))
        r = make_record()
        del r["spend"]["estimated"]
        self.assertTrue(schema.validate_record(r))

    def test_units_completed_is_derived_from_the_checks(self):
        r = make_record(units=(3, 2))
        self.assertEqual(schema.validate_record(r), [])
        r["metrics"]["units_completed"] = 3
        self.assertTrue(any("units_completed" in e for e in schema.validate_record(r)))

    def test_schema_version_is_one(self):
        r = copy.deepcopy(make_record())
        r["schema"] = 2
        self.assertTrue(schema.validate_record(r))

    def test_a_non_dict_is_invalid(self):
        self.assertTrue(schema.validate_record([]))


if __name__ == "__main__":
    unittest.main()
