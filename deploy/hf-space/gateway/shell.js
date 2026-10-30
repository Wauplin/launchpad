const el = id => document.getElementById(id);
async function refresh() {
  const buttons = [el('refresh'), el('retry')];
  buttons.forEach(b => b.disabled = true);
  try {
    const response = await fetch('/_space/status', {cache: 'no-store'});
    if (!response.ok) throw new Error();
    const state = await response.json();
    // All destination links are generated and validated by the supervisor.
    el('settings').href = el('fix').href = state.settings;
    el('status').textContent = state.ready ? 'Private at startup · Ready' : 'Startup paused · No agents started';
    el('detail').textContent = state.message;
    if (!state.ready) {
      el('label').textContent = 'Startup paused';
      el('title').textContent = 'One check before we begin.';
      el('intro').textContent = 'Launchpad stays stopped until this deployment can safely start. Follow the guidance below, then restart the Space in Hub settings.';
    }
    document.body.classList.toggle('ready', state.ready);
    if (state.ready && !el('app').src) {
      const route = new URLSearchParams(location.search).get('route') || '/agents';
      el('app').src = route.startsWith('/') && !route.startsWith('//') && !route.startsWith('/_space') ? route : '/agents';
    }
  } catch {
    el('detail').textContent = 'The Space is reconnecting. Check status again in a moment.';
  } finally {
    buttons.forEach(b => b.disabled = false);
  }
}
el('refresh').addEventListener('click', refresh);
el('retry').addEventListener('click', refresh);
refresh();
