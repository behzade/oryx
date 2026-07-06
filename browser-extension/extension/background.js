const CONTEXT_MENU_PAGE = "open-page-in-oryx";
const CONTEXT_MENU_LINK = "open-link-in-oryx";
const NATIVE_MESSAGING_HOST = "dev.oryx.app";

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

  if (await openWithNativeHost(url, source)) {
    return;
  }

  await openDeepLink(url);
}

function isSupportedUrl(url) {
  return /^https?:\/\//i.test(url);
}

async function openDeepLink(url) {
  const deepLink = `oryx://open?url=${encodeURIComponent(url)}`;
  await chrome.tabs.create({ url: deepLink, active: true });
}

async function openWithNativeHost(url, source) {
  try {
    const response = await sendNativeMessage({
      action: "open_url",
      source,
      url
    });
    return response?.ok === true;
  } catch (error) {
    console.warn("Oryx native messaging handoff failed; falling back to deep link.", error);
    return false;
  }
}

function sendNativeMessage(message) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendNativeMessage(NATIVE_MESSAGING_HOST, message, (response) => {
      const error = chrome.runtime.lastError;
      if (error) {
        reject(new Error(error.message));
        return;
      }

      if (response?.ok === false) {
        reject(new Error(response.error || "Native host rejected the request."));
        return;
      }

      resolve(response);
    });
  });
}
