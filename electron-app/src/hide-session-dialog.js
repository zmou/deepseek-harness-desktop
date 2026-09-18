'use strict'

/**
 * 隐藏 dsh 前端的 Session 导出反馈弹窗（规格 §7.7）。
 *
 * dsh 的 `SessionLogDownloadDialog`（`@deepseek-ai/dsh-client-ui-primitives` 的 `Modal`）
 * 通过 React Portal 渲染到 `document.body`，且为乐观 UI——点击下载后无论成败都会弹出
 * 「Session 导出已开始下载」提示，与本壳真实的「另存为」流程重复且误导用户。
 *
 * 在不修改 dsh 源码的前提下用 ARIA 属性选择器隐藏它：dsh `Modal` 渲染出
 * `role="dialog"` + `aria-modal="true"` + `aria-label={title}`，三个状态
 * （preparing / success / error）的标题都含 "Session"，而 ARIA 属性比 CSS Modules
 * 的 hash 类名稳定。整块隐藏可连同半透明遮罩一起去掉，避免留下灰幕挡住 UI。
 *
 * 两条腿都用，与 Tauri 版行为对齐：
 * - `insertCSS`：CSS 规则对后插入的节点同样生效（`:has()` 覆盖遮罩 wrapper）；
 * - `executeJavaScript`：复刻 Tauri 的 MutationObserver 兜底，处理 React Portal
 *   在 DOMContentLoaded 之后才创建 dialog 的场景。
 *
 * 本文件的常量必须与 tauri-app/src-tauri/src/main.rs 的 `HIDE_SESSION_DIALOG_SCRIPT`
 * 保持一致，避免两版行为漂移（规格 §13 R3）。
 */

const HIDE_SESSION_DIALOG_CSS = [
  '[role="dialog"][aria-modal="true"][aria-label*="Session" i]{display:none!important}',
  ':has(>[role="dialog"][aria-modal="true"][aria-label*="Session" i]){display:none!important}',
].join('\n')

const HIDE_SESSION_DIALOG_SCRIPT = `
(function () {
  function injectHideRule() {
    if (document.getElementById('dsh-desktop-hide-session-dialog')) return;
    var style = document.createElement('style');
    style.id = 'dsh-desktop-hide-session-dialog';
    style.textContent = '[role="dialog"][aria-modal="true"][aria-label*="Session" i]{display:none!important}\\n:has(>[role="dialog"][aria-modal="true"][aria-label*="Session" i]){display:none!important}';
    document.head.appendChild(style);
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', injectHideRule);
  } else {
    injectHideRule();
  }
  function hideExistingDialogs() {
    var dialogs = document.querySelectorAll('[role="dialog"][aria-modal="true"]');
    for (var i = 0; i < dialogs.length; i++) {
      var label = dialogs[i].getAttribute('aria-label') || '';
      if (label.toLowerCase().indexOf('session') !== -1) {
        dialogs[i].style.display = 'none';
        var p = dialogs[i].parentElement;
        if (p) p.style.display = 'none';
      }
    }
  }
  var observer = new MutationObserver(function () {
    hideExistingDialogs();
  });
  function startObserver() {
    if (document.body) {
      observer.observe(document.body, { childList: true, subtree: true });
      hideExistingDialogs();
    }
  }
  if (document.body) {
    startObserver();
  } else {
    document.addEventListener('DOMContentLoaded', startObserver);
  }
})();
`

/**
 * 把隐藏规则挂到指定 webContents；每次页面加载完成都重新注入。
 * @param {import('electron').WebContents} webContents
 */
function attach(webContents) {
  webContents.on('did-finish-load', () => {
    webContents.insertCSS(HIDE_SESSION_DIALOG_CSS).catch(() => {
      /* 注入失败不影响主流程 */
    })
    webContents.executeJavaScript(HIDE_SESSION_DIALOG_SCRIPT, true).catch(() => {
      /* 注入失败不影响主流程 */
    })
  })
}

module.exports = { HIDE_SESSION_DIALOG_CSS, HIDE_SESSION_DIALOG_SCRIPT, attach }
