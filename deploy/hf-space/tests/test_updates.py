"""Run from deploy/hf-space: python -m unittest discover -s tests -v."""
import json
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, Mock, patch

from aiohttp import web
from supervisor import updates


class UpdateChecks(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.manifest = {
            'image': updates.CHANNEL['image'] + '@sha256:' + 'b' * 64,
            'launchpad': 'b' * 40, 'adapter': '0.1.0', 'tools': {},
        }
        self.code = 200
        self.requests = []

        async def manifest(request):
            self.requests.append(dict(request.headers))
            return web.Response(status=self.code, text=json.dumps(self.manifest), headers={'ETag': '"candidate"'})

        app = web.Application()
        app.router.add_get('/manifest', manifest)
        self.runner = web.AppRunner(app)
        await self.runner.setup()
        site = web.TCPSite(self.runner, '127.0.0.1', 0)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        self.url_patch = patch.object(updates, 'MANIFEST', f'http://127.0.0.1:{port}/manifest')
        self.url_patch.start()
        self.addCleanup(self.url_patch.stop)
        self.addAsyncCleanup(self.runner.cleanup)
        with patch.object(updates, 'HfApi'), patch.dict(updates.os.environ, {'LP_SPACE_IMAGE': updates.CHANNEL['image'] + '@sha256:' + 'a' * 64}):
            self.updater = updates.Updates(SimpleNamespace(space='owner/space'), 'unused', Mock(), Mock(), {'launchpad': 'a' * 40})

    async def test_main_commit_becomes_candidate_without_a_release(self):
        await self.updater.check()
        self.assertEqual(self.updater.candidate, self.manifest)
        self.assertIsNotNone(self.updater.checked_at)
        self.updater.api.create_commit.assert_not_called()

    async def test_same_commit_rebuild_is_not_a_new_version(self):
        self.manifest['launchpad'] = 'a' * 40
        await self.updater.check()
        self.assertIsNone(self.updater.candidate)

    async def test_not_modified_clears_transient_error(self):
        await self.updater.check()
        self.updater.error = 'Temporary network failure'
        self.code = 304
        await self.updater.check()
        self.assertEqual(self.requests[-1]['If-None-Match'], '"candidate"')
        self.assertIsNone(self.updater.error)
        self.assertEqual(self.updater.candidate, self.manifest)

    async def test_bad_manifest_and_unavailable_channel_preserve_candidate(self):
        await self.updater.check()
        candidate = self.updater.candidate
        self.manifest['image'] = 'untrusted/image:latest'
        with self.assertRaises(ValueError):
            await self.updater.check()
        self.assertEqual(self.updater.candidate, candidate)
        self.code = 503
        with self.assertRaises(Exception):
            await self.updater.check()
        self.assertEqual(self.updater.candidate, candidate)

    async def test_changed_candidate_refuses_write(self):
        with self.assertRaisesRegex(ValueError, 'changed'):
            await self.updater.apply({'checkpoint': 'c' * 32}, 'previously-reviewed-image')
        self.updater.api.create_commit.assert_not_called()
        self.updater.recovery.bucket.put.assert_not_called()

    async def test_confirmed_update_pins_image_and_expected_parent(self):
        image = self.updater.installed['image']
        self.updater.api.space_info.return_value = SimpleNamespace(sha='d' * 40)
        self.updater.read_file = AsyncMock(return_value=updates.dockerfile(image))
        agents = [{'id': 's1', 'title': 'Fix the build'}]
        await self.updater.apply({'checkpoint': 'c' * 32}, self.manifest['image'], agents)
        call = self.updater.api.create_commit.call_args
        self.assertEqual(call.kwargs['parent_commit'], 'd' * 40)
        content = call.args[1][0].path_or_fileobj.getvalue().decode()
        self.assertEqual(content, updates.dockerfile(self.manifest['image']))
        self.assertEqual(self.updater.rollback_record['image'], image)
        self.assertEqual(self.updater.rollback_record['checkpoint'], 'c' * 32)
        # The fixed pre-update restore point is written before the update record and the Dockerfile commit.
        self.updater.recovery.bucket.save_pre_update.assert_called_once()
        # The resume list is durable before the commit, so the next boot can resume the stopped agents.
        stored = json.loads(self.updater.recovery.bucket.put.call_args.args[1])
        self.assertEqual(stored['agents'], agents)
        self.assertRegex(stored['request'], '^[0-9a-f]{32}$')
        self.assertEqual(stored['to'], 'b' * 40)

    async def test_background_check_cannot_replace_the_reviewed_image_mid_update(self):
        expected = self.manifest['image']
        def head(_space):
            self.updater.candidate = dict(self.manifest, image=updates.CHANNEL['image'] + '@sha256:' + 'e' * 64)
            return SimpleNamespace(sha='d' * 40)
        self.updater.api.space_info.side_effect = head
        self.updater.read_file = AsyncMock(return_value=updates.dockerfile(self.updater.installed['image']))
        await self.updater.apply({'checkpoint': 'c' * 32}, expected)
        content = self.updater.api.create_commit.call_args.args[1][0].path_or_fileobj.getvalue().decode()
        self.assertEqual(content, updates.dockerfile(expected))
        self.assertEqual(self.updater.rollback_record['candidate'], expected)

    async def test_custom_dockerfile_is_preserved(self):
        self.updater.api.space_info.return_value = SimpleNamespace(sha='d' * 40)
        self.updater.read_file = AsyncMock(return_value='FROM custom/image\n')
        with self.assertRaisesRegex(ValueError, 'custom changes'):
            await self.updater.apply({'checkpoint': 'c' * 32}, self.manifest['image'])
        self.updater.api.create_commit.assert_not_called()
        self.updater.recovery.bucket.put.assert_not_called()
