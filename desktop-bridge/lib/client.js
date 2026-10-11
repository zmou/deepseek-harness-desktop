/**
 * dsh-desktop bridge — browser 半（页面内 client 插件）。
 *
 * 目标（desktop-completion-notification-spec.md §2.2/§4.4 / 用户已裁决的方案 2）：
 * 「当前查看 session」没有官方外部稳定信号；本插件在 dsh 前端自身 Cordis 上下文内
 * 运行（官方 client 插件机制，不改任何官方 bundle），订阅 `ctx.uiSession.current`
 * （mainView binding source，见 `@deepseek-ai/dsh-client-ui-session`），把「用户打开/
 * 切换到的 sessionId 变化」写入 `window.__dshDesktopViewed__` 队列，供桌面主进程
 * 周期收集（Tauri `eval_with_callback` / Electron `executeJavaScript`）。
 *
 * 安全边界（规格 §7）：插件只读前端既有状态并写入专用全局，不调用任意原生 API；
 * 主进程消费端校验 sessionId 形状后仅用于从未读集合移除。不包含任何会话内容。
 */

window.__ModuleLoader__.load({
  id: 'dsh-desktop-bridge',
  factory: (require) => {
    var module = { exports: {} }
    var exports = module.exports
    Object.defineProperty(exports, Symbol.toStringTag, { value: 'Module' })

    // 队列全局：主进程每次收集后清空；限长防止插件异常时无限膨胀。
    function ensureQueue() {
      var q = window.__dshDesktopViewed__
      if (!Array.isArray(q)) q = window.__dshDesktopViewed__ = []
      return q
    }
    function pushViewed(sessionId) {
      if (typeof sessionId !== 'string' || sessionId.length === 0) return
      var q = ensureQueue()
      if (q.length >= 500) q.length = 250 // 丢弃最旧
      // 去重：与队尾相同视为未变化
      if (q[q.length - 1] === sessionId) return
      q.push(sessionId)
    }

    function apply(ctx) {
      // 就绪标记：供桌面主进程一次性探测「桥是否已在页面内挂载」（诊断用，
      // 不参与业务逻辑；据此区分「桥未注入」与「尚无查看事件」两种情形）。
      window.__dshDesktopBridgeReady__ = true
      var uiSession = ctx && ctx.uiSession
      if (!uiSession || !uiSession.current) {
        window.__dshDesktopBridgeState__ = 'no-ui-session'
        return
      }
      window.__dshDesktopBridgeState__ = 'attached'
      var current = uiSession.current
      var last = current && current.value ? current.value.sessionId : undefined
      var onChange = function () {
        var value = current.value
        var sid = value ? value.sessionId : undefined
        if (sid && sid !== last) {
          last = sid
          pushViewed(sid)
        }
      }
      if (typeof uiSession.current.subscribe === 'function') {
        uiSession.current.subscribe(onChange)
      }
      // 初始值：若当前已在查看某会话，也上报（覆盖「应用启动后用户已在看」场景）
      if (last) pushViewed(last)
      // eslint-disable-next-line no-console
      console.log('[dsh-desktop-bridge] attached', last || '(none)')
    }

    module.exports = { apply, inject: ['uiSession', 'remote', 'sessions'] }
    return module.exports
  },
})