"""Latest-only checkpoint planning and self-contained credential objects."""
import base64
import os
import unittest

from backup.store import plan
from vault.codec import Vault, key_id, seal_id


class SlotPlanning(unittest.TestCase):
    def test_changed_objects_alternate_slots_and_unchanged_ones_stay(self):
        entries, uploads = plan({'state/state.db': b'one', 'config/config.toml': b'c'}, None)
        self.assertEqual(sorted(uploads), ['config/config.toml', 'state/state.db'])
        self.assertEqual(entries['state/state.db']['path'], 'space/a/state/state.db')
        first = {'files': entries}
        entries, uploads = plan({'state/state.db': b'two', 'config/config.toml': b'c'}, first)
        self.assertEqual(uploads, ['state/state.db'])
        self.assertEqual(entries['state/state.db']['path'], 'space/b/state/state.db')
        self.assertEqual(entries['config/config.toml'], first['files']['config/config.toml'])
        entries, _ = plan({'state/state.db': b'three'}, {'files': entries})
        self.assertEqual(entries['state/state.db']['path'], 'space/a/state/state.db')
        # Objects not captured this time (the agent archive) are carried over.
        self.assertIn('config/config.toml', entries)

    def test_idle_machine_uploads_nothing(self):
        entries, _ = plan({'state/state.db': b'same'}, None)
        again, uploads = plan({'state/state.db': b'same'}, {'files': entries})
        self.assertEqual((again, uploads), (entries, []))

    def test_restored_pre_update_objects_are_never_overwritten(self):
        previous = {'files': {'state/state.db': {'sha256': 'x', 'bytes': 1, 'path': 'space/pre-update/state/state.db'}}}
        entries, _ = plan({'state/state.db': b'new'}, previous)
        self.assertEqual(entries['state/state.db']['path'], 'space/a/state/state.db')


class CredentialObjects(unittest.TestCase):
    def test_rotated_copy_keeps_the_seal_and_opens_only_with_its_key(self):
        old = Vault(base64.b64encode(os.urandom(32)).decode(), 'install')
        new = Vault(base64.b64encode(os.urandom(32)).decode(), 'install')
        sealed = old.seal_object({'files': {'a': 'b'}})
        copy = new.seal_object(old.open_object(sealed), seal_id(sealed))
        self.assertEqual(new.open_object(copy), {'files': {'a': 'b'}})
        self.assertEqual(seal_id(copy), seal_id(sealed))
        self.assertNotEqual(key_id(copy), key_id(sealed))
        with self.assertRaises(ValueError):
            old.open_object(copy)

    def test_installation_is_bound(self):
        key = base64.b64encode(os.urandom(32)).decode()
        sealed = Vault(key, 'one').seal_object({'x': 1})
        with self.assertRaises(Exception):
            Vault(key, 'two').open_object(sealed)


if __name__ == '__main__':
    unittest.main()
