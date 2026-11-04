"""Stop, update and resume orchestration. Run from deploy/hf-space: python -m unittest discover -s tests -v."""
import json
import time
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

from supervisor import updates


class Daemon:
    """In-memory stand-in for the Launchpad session API used by the updater."""

    def __init__(self, statuses, terminals=(), refuse=()):
        self.sessions = {id: {'id': id, 'title': f'Agent {id}', 'status': status} for id, status in statuses.items()}
        self.terminals = [{'alive': alive} for alive in terminals]
        self.refuse = set(refuse)
        self.calls = []

    async def api(self, method, path, body=None, detail=False):
        self.calls.append((method, path, body))
        if path.startswith('/api/sessions?'):
            return list(self.sessions.values())
        if path == '/api/terminals':
            return self.terminals
        id, verb = path.split('/')[3:5]
        if id in self.refuse:
            raise ValueError('Wait for the current turn, question and outstanding commands to finish.')
        session = self.sessions[id]
        if verb == 'interrupt':
            session['status'] = 'stopped'
        elif verb == 'resume':
            assert body == {'continue_turn': True}
            session['status'] = 'working'
        return {'ok': True}


class Bucket:
    def __init__(self):
        self.objects = {}

    def exists(self, name):
        return name in self.objects

    def put(self, name, data):
        self.objects[name] = data

    def read_json(self, name):
        return json.loads(self.objects[name]) if name in self.objects else None


class Orchestration(unittest.IsolatedAsyncioTestCase):
    async def test_plan_lists_running_agents_and_blockers(self):
        daemon = Daemon({'a': 'working', 'b': 'background', 'c': 'waiting', 'd': 'quota_paused'}, terminals=[True, False])
        plan = await updates.plan(daemon)
        self.assertEqual([a['id'] for a in plan['running']], ['a', 'b'])
        self.assertEqual(plan['terminals'], 1)
        self.assertEqual(len(plan['blockers']), 1)
        self.assertIn('Agent d', plan['blockers'][0])

    async def test_stop_interrupts_only_the_running_agents(self):
        daemon = Daemon({'a': 'working', 'b': 'starting', 'c': 'waiting'})
        left = await updates.stop_agents(daemon, ['a', 'b'], poll=0)
        self.assertEqual(left, [])
        self.assertEqual([c[1] for c in daemon.calls if c[0] == 'POST'], ['/api/sessions/a/interrupt', '/api/sessions/b/interrupt'])
        self.assertEqual(daemon.sessions['c']['status'], 'waiting')

    async def test_stop_gives_up_after_the_timeout(self):
        daemon = Daemon({'a': 'working'}, refuse={'a'})
        self.assertEqual(await updates.stop_agents(daemon, ['a'], timeout=0, poll=0), ['a'])

    async def test_resume_continues_stopped_and_interrupted_and_reports_the_rest(self):
        daemon = Daemon({'a': 'stopped', 'b': 'interrupted', 'c': 'waiting', 'd': 'stopped', 'e': 'closed'}, refuse={'d'})
        agents = [{'id': id, 'title': f'Agent {id}'} for id in 'abcdef']
        result = await updates.resume_agents(daemon, agents)
        self.assertEqual([a['id'] for a in result['resumed']], ['a', 'b'])
        self.assertEqual([a['id'] for a in result['finished']], ['c'])
        self.assertEqual({a['id']: a['error'] for a in result['failed']}, {
            'd': 'Wait for the current turn, question and outstanding commands to finish.',
            'e': 'Session is closed', 'f': 'Session not found'})


class AfterUpdate(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.old = updates.CHANNEL['image'] + '@sha256:' + 'a' * 64
        self.new = updates.CHANNEL['image'] + '@sha256:' + 'b' * 64
        self.bucket = Bucket()
        with patch.object(updates, 'HfApi'), patch.dict(updates.os.environ, {'LP_SPACE_IMAGE': self.new}):
            self.updater = updates.Updates(SimpleNamespace(space='owner/space'), 'unused', Mock(), SimpleNamespace(bucket=self.bucket), {'launchpad': 'b' * 40})
        self.updater.rollback_record = {'image': self.old, 'candidate': self.new, 'generation': 'c' * 32, 'at': time.time(),
                                        'request': 'e' * 32, 'from': 'a' * 40, 'to': 'b' * 40,
                                        'agents': [{'id': 'a', 'title': 'Agent a'}]}

    async def test_new_version_claims_once_then_resumes(self):
        daemon = Daemon({'a': 'stopped'})
        await self.updater.finish(daemon)
        self.assertEqual(json.loads(self.bucket.objects[updates.RESUMED])['request'], 'e' * 32)
        self.assertEqual(self.updater.result['outcome'], 'updated')
        self.assertEqual([a['id'] for a in self.updater.result['resumed']], ['a'])
        self.assertEqual(self.updater.result['pending'], [])
        # A later restart of the same version never resumes again.
        daemon.sessions['a']['status'] = 'stopped'
        self.updater.result = None
        await self.updater.finish(daemon)
        self.assertIsNone(self.updater.result)
        self.assertEqual(sum(1 for c in daemon.calls if c[1].endswith('/resume')), 1)

    async def test_claim_failure_resumes_nothing(self):
        daemon = Daemon({'a': 'stopped'})
        self.bucket.put = Mock(side_effect=RuntimeError('bucket unavailable'))
        with self.assertRaises(RuntimeError):
            await self.updater.finish(daemon)
        self.assertFalse(any(c[1].endswith('/resume') for c in daemon.calls))

    async def test_rollback_boot_reports_and_resumes_on_the_previous_version(self):
        self.updater.installed['image'] = self.old
        await self.updater.finish(Daemon({'a': 'interrupted'}))
        self.assertEqual(self.updater.result['outcome'], 'rolled_back')
        self.assertEqual(len(self.updater.result['resumed']), 1)

    async def test_late_boot_does_not_catch_up(self):
        daemon = Daemon({'a': 'stopped'})
        self.updater.rollback_record['at'] = time.time() - updates.RESUME_WINDOW - 60
        await self.updater.finish(daemon)
        self.assertEqual([a['id'] for a in self.updater.result['failed']], ['a'])
        self.assertFalse(any(c[1].endswith('/resume') for c in daemon.calls))

    async def test_records_without_a_request_are_ignored(self):
        del self.updater.rollback_record['request']
        await self.updater.finish(Daemon({'a': 'stopped'}))
        self.assertIsNone(self.updater.result)
        self.assertEqual(self.bucket.objects, {})
