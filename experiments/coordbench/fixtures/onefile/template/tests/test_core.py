import unittest

from onefile import core


class TestCore(unittest.TestCase):
    def test_the_defaults_are_rendered_one_line_per_option(self):
        lines = core.render().splitlines()
        self.assertIn("host = localhost", lines)
        self.assertIn("port = 8080", lines)

    def test_there_is_a_header_line_before_the_options(self):
        first = core.render().splitlines()[0]
        self.assertIn("settings", first)
        self.assertNotIn(" = ", first)

    def test_overrides_replace_defaults(self):
        lines = core.render({"port": 9000}).splitlines()
        self.assertIn("port = 9000", lines)
        self.assertIn("host = localhost", lines)

    def test_an_unknown_override_raises_key_error(self):
        with self.assertRaises(KeyError):
            core.render({"nope": 1})

    def test_options_are_in_alphabetical_order(self):
        names = [line.split(" = ")[0] for line in core.render().splitlines()[1:]]
        self.assertEqual(names, sorted(names))

    def test_format_value_by_kind(self):
        self.assertEqual(core.format_value("text", "x"), "x")
        self.assertEqual(core.format_value("int", 7.0), "7")
        with self.assertRaises(ValueError):
            core.format_value("nope", 1)

    def test_parse_config_reads_lines_and_rejects_bad_ones(self):
        self.assertEqual(core.parse_config("# c\n\nport = 1\nhost=h\n"), {"port": "1", "host": "h"})
        with self.assertRaises(ValueError):
            core.parse_config("port 1")
        with self.assertRaises(ValueError):
            core.parse_config("nope = 1")

    def test_describe_lists_every_option(self):
        text = core.describe()
        self.assertIn("host (text, default localhost)", text)
        self.assertIn("port (int, default 8080)", text)


if __name__ == "__main__":
    unittest.main()
