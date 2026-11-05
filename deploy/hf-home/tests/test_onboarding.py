import unittest

from onboarding import SKIPPED, Setup, claude_token, derive_key, device_prompt, github_refused, key_opens, last_line, noreply_email, seal, unique_id, unseal


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
        out = f"\x1b[2GYour OAuth token:\r\n\x1b[2G\x1b[1m{token}\x1b[22m\r\n\r\n\x1b[2GStore\x1b[8Gthis token securely."
        self.assertEqual(claude_token(out), token)
        self.assertIsNone(claude_token("Paste code here if prompted >"))
        # Other token versions, and a token wrapped over two lines.
        newer = token.replace("oat01", "oat02")
        self.assertEqual(claude_token(f"{newer[:60]}\r\n  {newer[60:]}\r\n Store this token"), newer)

    def test_claude_token_partial(self):
        token = "sk-ant-oat01-" + "Ab_9-" * 20
        # A token cut by a read boundary is not taken until the next line or the exit.
        self.assertIsNone(claude_token(f"Your OAuth token:\r\n{token[:40]}"))
        self.assertEqual(claude_token(f"Your OAuth token:\r\n{token}\r\n", complete=True), token)

    def test_last_line(self):
        self.assertEqual(last_line("Exchanging\r\n\x1b[2GOAuth error: timeout\r\n\r\n"), "OAuth error: timeout")
        self.assertEqual(last_line("token sk-ant-oat01-abc"), "token …")

    def test_nothing_yet(self):
        self.assertEqual(device_prompt(""), (None, None))


class Sealing(unittest.TestCase):
    def test_key_opens(self):
        import tempfile
        from pathlib import Path
        key, other = derive_key("correct horse battery"), derive_key("another key, longer")
        with tempfile.TemporaryDirectory() as d:
            files = [Path(d) / "a", Path(d) / "b"]
            self.assertIsNone(key_opens(key, files))
            files[0].write_bytes(seal(key, b"token"))
            self.assertTrue(key_opens(key, files))
            self.assertFalse(key_opens(other, files))

    def test_roundtrip(self):
        key = derive_key("correct horse battery")
        self.assertEqual(len(key), 32)
        self.assertEqual(key, derive_key("  correct horse battery\n"))
        self.assertNotEqual(key, derive_key("correct horse battery2"))
        self.assertIsNone(derive_key("short"))
        sealed = seal(key, b"hf_token")
        self.assertNotIn(b"hf_token", sealed)
        self.assertEqual(unseal(key, sealed), b"hf_token")
        with self.assertRaises(ValueError):
            unseal(key, b"plain")


class Email(unittest.TestCase):
    def test_noreply(self):
        self.assertEqual(noreply_email({"id": "42", "login": "octo"}), "42+octo@users.noreply.github.com")
        self.assertEqual(noreply_email({}), "")

    def test_github_refused(self):
        self.assertTrue(github_refused("gh: Bad credentials (HTTP 401)"))
        self.assertFalse(github_refused('Get "https://api.github.com/user": dial tcp: connection refused'))


class UniqueId(unittest.TestCase):
    def test_suffix(self):
        self.assertEqual(unique_id("codex", set()), "codex")
        self.assertEqual(unique_id("codex", {"codex", "codex-2"}), "codex-3")


class Steps(unittest.TestCase):
    def setup(self, **home):
        s = Setup.__new__(Setup)
        s.hf, s.key, s.github, s.marks = {"done": True}, b"k" * 32, {"done": True}, set()
        s.home = {k: {"done": v} for k, v in home.items()}
        return s

    def test_optional_steps_never_count(self):
        s = self.setup(ssh=True, identity=True, profile=True, folders=True)
        self.assertTrue(s.complete)
        self.assertEqual(sum(1 for x in s.steps() if x["required"] and not x["done"]), 0)
        s.home["folders"]["done"] = False
        self.assertFalse(s.complete)
        self.assertEqual([x["id"] for x in s.steps() if x["required"] and not x["done"]], ["folders"])

    def test_skipped(self):
        s = self.setup(runner=False, agent=True)
        s.marks = {SKIPPED + "runner", SKIPPED + "agent", SKIPPED + "identity"}
        steps = {x["id"]: x for x in s.steps()}
        self.assertTrue(steps["runner"]["skipped"])
        self.assertFalse(steps["agent"]["skipped"])  # Done wins.
        self.assertFalse(steps["identity"]["skipped"])  # Required steps cannot be skipped.


if __name__ == "__main__":
    unittest.main()
