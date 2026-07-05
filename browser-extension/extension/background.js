const CONTEXT_MENU_PAGE = "open-page-in-oryx";
const CONTEXT_MENU_LINK = "open-link-in-oryx";

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: CONTEXT_MENU_PAGE,
    title: "Open page in Oryx",
    contexts: ["page"]
  });

  chrome.contextMenus.create({
    id: CONTEXT_MENU_LINK,
    title: "Open link in Oryx",
    contexts: ["link"]
  });
});

chrome.action.onClicked.addListener(async (tab) => {
  await openUrlFromTab(tab, "toolbar");
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === CONTEXT_MENU_LINK && info.linkUrl) {
    await openUrl(info.linkUrl, "context-link");
    return;
  }

  if (info.menuItemId === CONTEXT_MENU_PAGE) {
    await openUrlFromTab(tab, "context-page");
  }
});

async function openUrlFromTab(tab, source) {
  if (!tab?.url) {
    return;
  }

  await openUrl(tab.url, source);
}

async function openUrl(url, source) {
  if (!isSupportedUrl(url)) {
    return;
  }

  await openDeepLink(url);
}

function isSupportedUrl(url) {
  return /^https?:\/\//i.test(url);
}

async function openDeepLink(url) {
  const deepLink = `oryx://open?url=${encodeURIComponent(url)}`;
  await chrome.tabs.create({ url: deepLink, active: false });
}
