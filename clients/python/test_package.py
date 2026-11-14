"""The published package (`klatch`) wraps klatch_client.py; its metadata must agree with itself."""
import os
import re
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


def read(*parts):
    with open(os.path.join(HERE, *parts), encoding="utf-8") as f:
        return f.read()


class PackageMetadata(unittest.TestCase):
    def test_the_package_version_matches_pyproject(self):
        declared = re.search(r'^version = "([^"]+)"', read("pyproject.toml"), re.M).group(1)
        inside = re.search(r'__version__ = "([^"]+)"', read("package", "__init__.py")).group(1)
        self.assertEqual(declared, inside)

    def test_the_package_exports_the_client_names(self):
        init = read("package", "__init__.py")
        for name in ("Client", "ApiError", "ProtocolError", "PROTOCOL_VERSION"):
            self.assertIn(name, init)

    def test_the_readme_says_it_is_a_pre_release_with_no_review(self):
        text = read("README.md").lower()
        for phrase in ("pre-release", "not for sensitive data", "not related to design-in-product/klatch"):
            self.assertIn(phrase, text)


if __name__ == "__main__":
    unittest.main()
