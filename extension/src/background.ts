// Service worker. Only the side panel opens from the toolbar button for now; the connection to Home, the badge and
// the GitHub layer come with the new extension.

chrome.runtime.onInstalled.addListener(() => {
  void chrome.storage.session.clear();
  void chrome.sidePanel.setPanelBehavior({ openPanelOnActionClick: true });
});
