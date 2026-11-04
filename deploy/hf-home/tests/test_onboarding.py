import unittest

import base64

from onboarding import claude_token, device_prompt, parse_key, seal, unique_id, unseal


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

    def test_claude_terminal_link(self):
        # The visible URL is cut at the terminal width; the OSC 8 hyperlink has all of it.
        url = "https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c&state=Wo4J"
        out = f"Browser didn't open?\x1b[23GUse the url\r\r\n\x1b]8;id=1ujkmf;{url}\x07https://claude.com/cai/oauth/auth\x1b]8;;\x07\r\r\n\x1b[2GPaste\x1b[8Gcode\x1b[13Ghere\x1b[30G>"
        self.assertEqual(device_prompt(out)[0], url)

    def test_claude_token(self):
        token = "sk-ant-oat01-" + "Ab_9-" * 20
        self.assertEqual(claude_token(f"\x1b[2GYour OAuth token:\r\n\x1b[2G\x1b[1m{token}\x1b[22m\r\n"), token)
        self.assertIsNone(claude_token("Paste code here if prompted >"))

    def test_nothing_yet(self):
        self.assertEqual(device_prompt(""), (None, None))


class Sealing(unittest.TestCase):
    def test_roundtrip_and_legacy(self):
        key = parse_key(base64.b64encode(bytes(range(32))).decode())
        sealed = seal(key, b"hf_token")
        self.assertNotIn(b"hf_token", sealed)
        self.assertEqual(unseal(key, sealed), b"hf_token")
        self.assertEqual(unseal(key, b"plain"), b"plain")  # From before encryption.
        with self.assertRaises(ValueError):
            unseal(None, sealed)
        self.assertIsNone(parse_key("c2hvcnQ="))


class UniqueId(unittest.TestCase):
    def test_suffix(self):
        self.assertEqual(unique_id("codex", set()), "codex")
        self.assertEqual(unique_id("codex", {"codex", "codex-2"}), "codex-3")


if __name__ == "__main__":
    unittest.main()
