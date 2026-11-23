import unittest

from aiohttp.test_utils import make_mocked_request
from multidict import CIMultiDict

from gateway.proxy import DEVICE_HEADER, EXTENSION_ORIGIN, Proxy, blocked, device_call, from_desktop, from_extension


def request(method="GET", path="/api/sessions", **headers):
    headers.setdefault("Host", "me-home.hf.space")
    return make_mocked_request(method, path, headers=CIMultiDict({k.replace("_", "-"): v for k, v in headers.items()}))


class Clients(unittest.TestCase):
    def test_desktop_is_explicit_and_never_with_an_origin(self):
        desktop = request(**{"X_Launchpad_Client": "desktop/0.1.0", DEVICE_HEADER: "s"})
        self.assertTrue(from_desktop(desktop))
        self.assertFalse(from_extension(desktop))
        self.assertTrue(device_call(desktop))
        page = request(**{"X_Launchpad_Client": "desktop/0.1.0", "Origin": "https://evil.example"})
        self.assertFalse(from_desktop(page))
        self.assertFalse(device_call(page))
        self.assertFalse(from_desktop(request(**{"X_Launchpad_Client": "browser/1"})))

    def test_extension_unchanged(self):
        self.assertTrue(from_extension(request(Origin=EXTENSION_ORIGIN)))
        self.assertTrue(from_extension(request(**{DEVICE_HEADER: "s"})))
        self.assertTrue(from_extension(request(Sec_Fetch_Site="none", Sec_Fetch_Mode="cors")))
        self.assertFalse(from_extension(request(Sec_Fetch_Site="same-origin", Sec_Fetch_Mode="cors")))

    def test_desktop_headers_carry_its_device_and_no_daemon_credential(self):
        proxy = Proxy("https://me-home.hf.space", "daemon-token")
        headers = proxy.headers(request(Authorization="Bearer hf_x", **{"X_Launchpad_Client": "desktop/0.1.0", DEVICE_HEADER: "s"}))
        self.assertEqual(headers[DEVICE_HEADER], "s")
        self.assertNotIn("Authorization", headers)
        self.assertNotIn("Origin", headers)
        page = proxy.headers(request(Origin="https://me-home.hf.space", **{DEVICE_HEADER: "s"}))
        self.assertNotIn(DEVICE_HEADER, page)
        self.assertEqual(page["Authorization"], "Bearer daemon-token")


class Devices(unittest.TestCase):
    def test_owner_browser_lists_and_revokes(self):
        self.assertFalse(blocked("GET", "/api/devices", False))
        self.assertFalse(blocked("DELETE", "/api/devices/abc", False))
        self.assertTrue(blocked("DELETE", "/api/devices/self", False))
        for method, path in [("POST", "/api/devices/pair"), ("POST", "/api/devices/reverse/approve"), ("GET", "/api/devices/pairing-info"),
                             ("POST", "/api/devices/extension"), ("POST", "/api/devices/desktop"), ("POST", "/api/devices"), ("GET", "/auth"),
                             ("POST", "/auth/reverse/start"), ("DELETE", "/api/devices/"), ("GET", "/api/window")]:
            self.assertTrue(blocked(method, path, False), (method, path))

    def test_devices_delete_only_themselves(self):
        self.assertFalse(blocked("DELETE", "/api/devices/self", True))
        for method, path in [("GET", "/api/devices"), ("DELETE", "/api/devices/abc"), ("POST", "/api/devices/desktop"), ("GET", "/auth")]:
            self.assertTrue(blocked(method, path, True), (method, path))
        self.assertFalse(blocked("GET", "/api/version", True))
        self.assertFalse(blocked("POST", "/api/extension/disconnect", True))


if __name__ == "__main__":
    unittest.main()
