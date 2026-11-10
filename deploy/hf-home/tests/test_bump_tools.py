import unittest

from bump_tools import plan


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


if __name__ == "__main__":
    unittest.main()
