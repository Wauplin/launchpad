import unittest

from bump_tools import github_versions, plan, pypi_versions


class Plan(unittest.TestCase):
    """The same decision as launchpad_setup::cli_update::plan."""

    def test_same_major(self):
        self.assertEqual(plan("2.1.311", "2.1.315"), ("2.1.315", None))
        self.assertEqual(plan("2.1.315", "2.1.315"), (None, None))
        self.assertEqual(plan("2.1.316", "2.1.315"), (None, None))

    def test_major(self):
        self.assertEqual(plan("2.1.311", "3.0.0"), (None, "3.0.0"))
        self.assertEqual(plan("2.1.311", "3.0.0", ["2.1.320", "2.2.0-beta.1", "3.0.0"]), ("2.1.320", "3.0.0"))

    def test_zero_minor_is_major(self):
        self.assertEqual(plan("0.166.0", "0.166.2"), ("0.166.2", None))
        self.assertEqual(plan("0.166.0", "0.167.0", ["0.166.3", "0.167.0"]), ("0.166.3", "0.167.0"))
        self.assertEqual(plan("0.0.3", "0.0.4"), (None, "0.0.4"))

    def test_pre_release(self):
        self.assertEqual(plan("2.1.311", "2.1.312-beta.1"), (None, None))


class Sources(unittest.TestCase):
    """Versions read from PyPI and GitHub answers (Monty, Ruff, slackline)."""

    def test_pypi(self):
        doc = {
            "info": {"version": "1.6.0"},
            "releases": {"1.0.0b1": [{"yanked": False}], "1.5.0": [{"yanked": True}], "1.6.0": [{"yanked": False}], "1.6.1": []},
        }
        self.assertEqual(pypi_versions(doc), ("1.6.0", ["1.0.0b1", "1.6.0"]))
        latest, versions = pypi_versions({"info": {"version": "0.18.0"}, "releases": {v: [{}] for v in ["0.17.5", "0.17.6", "0.18.0"]}})
        self.assertEqual(plan("0.17.5", latest, versions), ("0.17.6", "0.18.0"))

    def test_github(self):
        releases = [
            {"tag_name": "v2.0.0-rc.1", "prerelease": True},
            {"tag_name": "v1.6.0", "draft": True},
            {"tag_name": "v1.5.1"},
            {"tag_name": "v1.5.0"},
            {"tag_name": "nightly"},
        ]
        self.assertEqual(github_versions(releases), ("1.5.1", ["1.5.1", "1.5.0", "nightly"]))
        self.assertEqual(plan("1.5.0", *github_versions(releases)), ("1.5.1", None))
        self.assertEqual(github_versions([]), (None, []))


if __name__ == "__main__":
    unittest.main()
