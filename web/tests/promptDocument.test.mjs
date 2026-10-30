import test from 'node:test';
import assert from 'node:assert/strict';
import { editText, insertMention, replaceText, reconcile, PromptHistory } from '../src/lib/promptDocument.ts';
import { mentionAt } from '../src/lib/promptTriggers.ts';
const empty = () => ({ text: '', context_projects: [], github_refs: [], agent_traces: [], mentions: [], skills: [] });
const trace = (id = 'one', seq = 12) => ({ kind: 'trace', value: { session_id: id, through_seq: seq } });

test('same labels resolve to independent identities; unrelated substring is not a reference', () => {
  let d = insertMention(empty(), 0, 0, '#12', trace());
  d = insertMention(d, d.text.length, d.text.length, '#12', trace('two'));
  d = replaceText(d, 0, 3, '');
  assert.deepEqual(d.agent_traces.map(t => t.session_id), ['two']);
  d = editText(d, '#123');
  assert.equal(d.agent_traces.length, 0);
});
test('multiple occurrences keep one attachment until its last occurrence disappears', () => {
  let d = insertMention(empty(), 0, 0, '#trace', trace());
  d = insertMention(d, d.text.length, d.text.length, '#trace', trace());
  d = replaceText(d, 0, 6, '');
  assert.equal(d.agent_traces.length, 1);
  d = editText(d, '');
  assert.equal(d.agent_traces.length, 0);
});
test('manual context survives mention deletion; chip removal deletes linked occurrences', () => {
  let d = { ...empty(), agent_traces: [trace().value] };
  d = insertMention(d, 0, 0, '#trace', trace());
  assert.equal(editText(d, '').agent_traces.length, 1);
  const removed = reconcile(d, { ...d, agent_traces: [] });
  assert.equal(removed.text.trim(), '');
  assert.deepEqual(removed.mentions, []);
});
test('UTF-16 edits preserve ranges, partial replacements invalidate, undo restores context', () => {
  const d = insertMention({ ...empty(), text: '🌍 ' }, 3, 3, '#trace', trace());
  const history = new PromptHistory(d);
  const changed = replaceText(d, 0, 0, 'hello ');
  assert.equal(changed.mentions[0].start, 9);
  history.record(changed);
  history.record(replaceText(changed, 10, 12, 'xx'));
  assert.equal(history.current.agent_traces.length, 0);
  assert.equal(history.undo().agent_traces.length, 1);
  assert.equal(history.redo().agent_traces.length, 0);
});
test('persisted references preserve settings and fixed trace boundaries', () => {
  const d = insertMention(empty(), 0, 0, '#trace', trace('one', 42));
  const restored = JSON.parse(JSON.stringify(d));
  assert.deepEqual(editText(restored, `prefix ${restored.text}`).agent_traces, [{ session_id: 'one', through_seq: 42 }]);
  assert.equal(editText(restored, '').agent_traces.length, 0);
});
test('triggers open at boundaries and leave headings, email addresses and embedded paths alone', () => {
  for (const trigger of ['@', '/', '#']) assert.equal(mentionAt(trigger, 1).trigger, trigger);
  for (const text of ['user@host', 'src/lib', 'and/or', '# Title', '##', '/usr/bin']) assert.equal(mentionAt(text, text.length), null);
});
test('programmatic text changes preserve simultaneous manually added context', () => {
  const d = insertMention(empty(), 0, 0, '#trace', trace());
  const changed = reconcile(d, { ...d, text: '', context_projects: [{ project_id: 'manual', branch: 'feature', editable: false }] });
  assert.equal(changed.agent_traces.length, 0);
  assert.equal(changed.context_projects[0].branch, 'feature');
});
