import unittest

from onboarding import device_prompt, unique_id


class DevicePrompt(unittest.TestCase):
    """Real outputs of the fixed device logins (hf 2.0.0, gh 2.100.0, Codex 0.166.0)."""

    def test_hf_short_domain(self):
        out = "Ask the user to open https://hf.co/oauth/device in a browser and enter the code 98FI-C9IG. The code expires in 300 seconds. Waiting for authorization...\n"
        self.assertEqual(device_prompt(out), ("https://hf.co/oauth/device", "98FI-C9IG"))

    def test_gh(self):
        out = "\n! First copy your one-time code: FB08-C0AE\nOpen this URL to continue in your web browser: https://github.com/login/device\n"
        self.assertEqual(device_prompt(out), ("https://github.com/login/device", "FB08-C0AE"))

    def test_codex_colors(self):
        out = ("Welcome to Codex [v\x1b[90m0.166.0\x1b[0m]\n1. Open this link in your browser and sign in to your account\n"
               "   \x1b[94mhttps://auth.openai.com/codex/device\x1b[0m\n\n2. Enter this one-time code \x1b[90m(expires in 15 minutes)\x1b[0m\n"
               "   \x1b[94mZEDU-H80X7\x1b[0m\n")
        self.assertEqual(device_prompt(out), ("https://auth.openai.com/codex/device", "ZEDU-H80X7"))

    def test_nothing_yet(self):
        self.assertEqual(device_prompt(""), (None, None))


class UniqueId(unittest.TestCase):
    def test_suffix(self):
        self.assertEqual(unique_id("codex", set()), "codex")
        self.assertEqual(unique_id("codex", {"codex", "codex-2"}), "codex-3")


if __name__ == "__main__":
    unittest.main()
