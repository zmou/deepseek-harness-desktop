/**
 * dsh-desktop bridge — host 半（Node 侧）。
 *
 * 空 `apply`：仅使本包成为 Cordis 行进入 host Loader，真正的逻辑在浏览器半
 * `lib/client.js`（经 `__ModuleLoader__.load` 注册为 client 插件）。
 * 对齐 `@deepseek-ai/dsh-cordis-client-runner` 的「empty apply」模式。
 */
export function apply() {}