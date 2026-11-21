<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    discoverLanPeers,
    onLanSyncReceived,
    startLanSyncServer,
    stopLanSyncServer,
    syncWithLanPeer,
    testGeminiConnection,
  } from '../lib/api';
  import type { AppSettings, ImportResult, LanPeer, LanServerInfo } from '../lib/types';

  type ModalTab = 'sync' | 'backup' | 'settings';

  interface Props {
    open: boolean;
    initialTab?: ModalTab;
    settings: AppSettings;
    onClose: () => void;
    onSaveSettings: (next: AppSettings) => Promise<void>;
    onExportBackup: () => Promise<string | null>;
    onImportBackup: (jsonContent: string, overwrite: boolean) => Promise<ImportResult>;
    onBeforeSync?: () => Promise<void>;
    onSyncCompleted?: () => Promise<void>;
  }

  let {
    open,
    initialTab = 'settings',
    settings,
    onClose,
    onSaveSettings,
    onExportBackup,
    onImportBackup,
    onBeforeSync,
    onSyncCompleted,
  }: Props = $props();

  const MODEL_PRESETS = [
    'gemini-3.5-flash-lite',
    'gemini-3.5-flash',
    'gemini-3.6-flash',
    'gemini-3.7-flash',
    'gemini-3.8-flash',
    'gemini-4-hydrogen',
    'gemini-4-argon',
    'gemini-4-carbon',
    'gemini-4-neon',
  ] as const;

  let activeTab = $state<ModalTab>('settings');

  // Gemini 与主题设置状态
  let apiKey = $state('');
  let model = $state('gemini-3.8-flash');
  let baseUrl = $state('');
  let theme = $state('system');
  let showKey = $state(false);
  let overwrite = $state(false);
  let testing = $state(false);
  let testOkMsg = $state('');
  let testErrMsg = $state('');
  let statusMsg = $state('');
  let errorMsg = $state('');
  let busy = $state(false);
  let fileInputEl = $state<HTMLInputElement | null>(null);

  // 局域网 Wi-Fi 快传状态
  let serverInfo = $state<LanServerInfo | null>(null);
  let serverBusy = $state(false);
  let scanning = $state(false);
  let hasScannedOnce = $state(false);
  let peers = $state<LanPeer[]>([]);
  let peerAddress = $state('');
  let peerPin = $state('');
  let syncing = $state(false);
  let syncStatusMsg = $state('');
  let syncErrorMsg = $state('');

  let unlistenSync: (() => void) | null = null;
  let wasOpen = false;

  function formatPinDisplay(pin: string): string {
    return pin.length === 6 ? `${pin.slice(0, 3)} ${pin.slice(3)}` : pin;
  }

  function formatPlatformLabel(platform: string): string {
    if (platform === 'windows') return 'Windows';
    if (platform === 'android') return 'Android';
    return platform;
  }

  async function ensureSyncListener() {
    if (unlistenSync) return;
    try {
      unlistenSync = await onLanSyncReceived((evt) => {
        const imp = evt.localImport;
        syncErrorMsg = '';
        syncStatusMsg = `已接收来自「${evt.remoteDeviceName}」的同步：本机新增 ${imp.importedEntries} 篇、更新 ${imp.updatedEntries} 篇、报告 ${imp.importedReports} 条`;
        void onSyncCompleted?.();
      });
    } catch {
      // 忽略事件监听初始化异常
    }
  }

  async function cleanupLanServer() {
    unlistenSync?.();
    unlistenSync = null;
    if (serverInfo) {
      serverInfo = null;
      try {
        await stopLanSyncServer();
      } catch {
        // 忽略关闭时的清理错误
      }
    }
  }

  $effect(() => {
    if (open && !wasOpen) {
      wasOpen = true;
      activeTab = initialTab;
      apiKey = settings.geminiApiKey;
      model = settings.geminiModel || 'gemini-3.8-flash';
      baseUrl = settings.geminiBaseUrl;
      theme = settings.theme || 'system';
      testOkMsg = '';
      testErrMsg = '';
      statusMsg = '';
      errorMsg = '';
      syncStatusMsg = '';
      syncErrorMsg = '';
      hasScannedOnce = false;
      peers = [];
      void ensureSyncListener();
    } else if (!open && wasOpen) {
      wasOpen = false;
      void cleanupLanServer();
    }
  });

  onDestroy(() => {
    void cleanupLanServer();
  });

  function switchTab(tab: ModalTab) {
    activeTab = tab;
  }

  async function handleStartServer() {
    if (serverBusy) return;
    serverBusy = true;
    syncErrorMsg = '';
    syncStatusMsg = '';
    try {
      await onBeforeSync?.();
      await ensureSyncListener();
      serverInfo = await startLanSyncServer();
    } catch (e) {
      syncErrorMsg = String(e);
    } finally {
      serverBusy = false;
    }
  }

  async function handleStopServer() {
    if (serverBusy) return;
    serverBusy = true;
    try {
      await stopLanSyncServer();
      serverInfo = null;
    } catch (e) {
      syncErrorMsg = String(e);
    } finally {
      serverBusy = false;
    }
  }

  async function handleCopyServerAddress(addr = serverInfo?.address) {
    if (!addr) return;
    try {
      await navigator.clipboard.writeText(addr);
      syncStatusMsg = `已复制连接地址：${addr}`;
    } catch (e) {
      syncErrorMsg = String(e);
    }
  }

  async function handleDiscoverPeers() {
    if (scanning) return;
    scanning = true;
    syncErrorMsg = '';
    try {
      peers = await discoverLanPeers(1800);
      hasScannedOnce = true;
      if (peers.length === 1 && !peerAddress.trim()) {
        peerAddress = peers[0].address;
      }
    } catch (e) {
      syncErrorMsg = String(e);
    } finally {
      scanning = false;
    }
  }

  async function handleSyncWithPeer() {
    if (syncing) return;
    syncing = true;
    syncErrorMsg = '';
    syncStatusMsg = '';
    try {
      await onBeforeSync?.();
      const res = await syncWithLanPeer(peerAddress, peerPin);
      await onSyncCompleted?.();
      syncStatusMsg =
        `已与「${res.remoteDeviceName}」完成双向同步：` +
        `本机新增 ${res.localImport.importedEntries} 篇、更新 ${res.localImport.updatedEntries} 篇、报告 ${res.localImport.importedReports} 条；` +
        `对方新增 ${res.remoteImport.importedEntries} 篇、更新 ${res.remoteImport.updatedEntries} 篇`;
      peerPin = '';
    } catch (e) {
      syncErrorMsg = String(e);
    } finally {
      syncing = false;
    }
  }

  async function handleSave() {
    busy = true;
    errorMsg = '';
    statusMsg = '';
    try {
      await onSaveSettings({
        geminiApiKey: apiKey,
        geminiModel: model,
        geminiBaseUrl: baseUrl,
        theme,
      });
      statusMsg = '设置已保存';
    } catch (e) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }

  async function handleTestConnection() {
    if (testing) return;
    testing = true;
    testOkMsg = '';
    testErrMsg = '';
    try {
      const res = await testGeminiConnection(apiKey, model, baseUrl);
      const preview = res.replyPreview ? ` · 回复：${res.replyPreview}` : '';
      testOkMsg = `模型 ${res.model} 可用（耗时 ${res.latencyMs} 毫秒${preview}）`;
    } catch (e) {
      testErrMsg = String(e);
    } finally {
      testing = false;
    }
  }

  async function handleExport() {
    busy = true;
    errorMsg = '';
    statusMsg = '';
    try {
      const savedPath = await onExportBackup();
      if (savedPath) {
        statusMsg = `备份已保存至：${savedPath}`;
      }
    } catch (e) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }

  async function handleFileSelected(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    busy = true;
    errorMsg = '';
    statusMsg = '';
    try {
      const json = await file.text();
      const res = await onImportBackup(json, overwrite);
      statusMsg = `导入完成：新增 ${res.importedEntries} 篇，更新 ${res.updatedEntries} 篇，跳过 ${res.skippedEntries} 篇，报告 ${res.importedReports} 条`;
    } catch (err) {
      errorMsg = String(err);
    } finally {
      input.value = '';
      busy = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (open && e.key === 'Escape') {
      onClose();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
  <div class="backdrop" role="presentation" onclick={onClose}>
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-label="快传、备份与设置"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
    >
      <header class="modal-head">
        <h2 class="modal-title">快传 · 备份 · 设置</h2>
        <button type="button" class="close-btn" onclick={onClose} aria-label="关闭">✕</button>
      </header>

      <nav class="tab-bar" aria-label="设置分类">
        <button
          type="button"
          class="tab-btn"
          class:active={activeTab === 'sync'}
          onclick={() => switchTab('sync')}
        >局域网快传</button>
        <button
          type="button"
          class="tab-btn"
          class:active={activeTab === 'backup'}
          onclick={() => switchTab('backup')}
        >本地备份</button>
        <button
          type="button"
          class="tab-btn"
          class:active={activeTab === 'settings'}
          onclick={() => switchTab('settings')}
        >Gemini 与设置</button>
      </nav>

      <div class="modal-body">
        {#if activeTab === 'sync'}
          <section class="block">
            <h3 class="block-title">方式一：本机开启快传服务（等待对方连接）</h3>
            <p class="hint">
              快传为双向智能合并：任意一端发起一次同步，双端即同时更新为最新，无需两边各同步一次。仅同步日记与 AI 历史报告，API Key 仅留在本机。
            </p>

            {#if serverInfo}
              <div class="server-card">
                <div class="pin-box">
                  <span class="pin-label">6 位配对码</span>
                  <strong class="pin-code">{formatPinDisplay(serverInfo.pin)}</strong>
                </div>
                <div class="addr-row">
                  <div class="addr-meta">
                    <span class="field-label">本机地址（{serverInfo.deviceName}）</span>
                    <code class="addr-text">{serverInfo.address}</code>
                  </div>
                  <button
                    type="button"
                    class="mini-btn"
                    onclick={() => handleCopyServerAddress(serverInfo?.address)}
                  >
                    复制地址
                  </button>
                </div>
                {#if serverInfo.addresses.length > 1}
                  <div class="alt-addrs">
                    <span class="field-label">其他候选局域网地址（点击复制）：</span>
                    <div class="alt-list">
                      {#each serverInfo.addresses.slice(1) as altAddr}
                        <button
                          type="button"
                          class="mini-btn"
                          onclick={() => handleCopyServerAddress(altAddr)}
                        >
                          {altAddr}
                        </button>
                      {/each}
                    </div>
                  </div>
                {/if}
                <button
                  type="button"
                  class="secondary-btn"
                  disabled={serverBusy}
                  onclick={handleStopServer}
                >
                  停止服务
                </button>
              </div>
            {:else}
              <button
                type="button"
                class="primary-btn"
                disabled={serverBusy}
                onclick={handleStartServer}
              >
                {serverBusy ? '正在启动…' : '开启局域网快传'}
              </button>
            {/if}
          </section>

          <section class="block">
            <div class="row-between">
              <h3 class="block-title">方式二：连接对方设备并双向同步</h3>
              <button
                type="button"
                class="mini-btn"
                disabled={scanning}
                onclick={handleDiscoverPeers}
              >
                {scanning ? '扫描中…' : '扫描设备'}
              </button>
            </div>
            <p class="hint">
              手机用流量开热点给电脑时，双方在同一热点网段内可直连互通；若手机开启了代理/VPN，请先开启“绕过局域网”或暂时关闭代理。
            </p>

            {#if peers.length > 0}
              <div class="peer-list">
                {#each peers as peer}
                  <button
                    type="button"
                    class="peer-item"
                    class:selected={peerAddress.trim() === peer.address}
                    onclick={() => (peerAddress = peer.address)}
                  >
                    <span class="peer-main">
                      <strong>{peer.deviceName}</strong>
                      <span class="peer-tag">{formatPlatformLabel(peer.platform)}</span>
                    </span>
                    <span class="peer-addr">{peer.address}</span>
                  </button>
                {/each}
              </div>
            {:else if hasScannedOnce && !scanning}
              <p class="hint">
                暂未扫描到已开启快传的设备。请确认对方已点击「开启局域网快传」，或直接在下方手动填写对方地址。
              </p>
            {/if}

            <label class="field">
              <span class="field-label">对方地址（IP:端口）</span>
              <input
                class="text-input"
                type="text"
                bind:value={peerAddress}
                placeholder="例如 192.168.1.105:17580"
              />
            </label>

            <label class="field">
              <span class="field-label">6 位配对码</span>
              <input
                class="text-input pin-input"
                type="text"
                inputmode="numeric"
                maxlength="6"
                bind:value={peerPin}
                placeholder="输入对方界面显示的 6 位数字"
              />
            </label>

            <button
              type="button"
              class="primary-btn"
              disabled={syncing || !peerAddress.trim() || peerPin.trim().length !== 6}
              onclick={handleSyncWithPeer}
            >
              {syncing ? '正在双向同步…' : '立即双向同步'}
            </button>
          </section>

          {#if syncStatusMsg}
            <p class="msg-ok">{syncStatusMsg}</p>
          {/if}
          {#if syncErrorMsg}
            <p class="msg-err">{syncErrorMsg}</p>
          {/if}
        {:else if activeTab === 'backup'}
          <section class="block">
            <h3 class="block-title">本地数据备份与恢复（.json）</h3>
            <p class="hint">
              单文件包含全部日记正文与历史 Gemini 分析报告（不含 API Key），可在 Windows 与 Android 间手动迁移。
            </p>

            <div class="backup-actions">
              <button type="button" class="secondary-btn" disabled={busy} onclick={handleExport}>
                一键导出 .json 备份
              </button>
              <button
                type="button"
                class="secondary-btn"
                disabled={busy}
                onclick={() => fileInputEl?.click()}
              >
                导入 .json 备份恢复
              </button>
              <input
                bind:this={fileInputEl}
                type="file"
                accept=".json,application/json"
                class="hidden-file"
                onchange={handleFileSelected}
              />
            </div>

            <label class="check-row">
              <input type="checkbox" bind:checked={overwrite} />
              <span>全量覆盖本地现有数据（不勾选则按日期智能合并，保留较新内容）</span>
            </label>
          </section>

          {#if statusMsg}
            <p class="msg-ok">{statusMsg}</p>
          {/if}
          {#if errorMsg}
            <p class="msg-err">{errorMsg}</p>
          {/if}
        {:else}
          <section class="block">
            <h3 class="block-title">Gemini 大模型接口</h3>

            <label class="field">
              <span class="field-label">API Key（仅存本地私有数据库，不参与同步）</span>
              <div class="key-row">
                <input
                  class="text-input"
                  type={showKey ? 'text' : 'password'}
                  bind:value={apiKey}
                  placeholder="AIza..."
                />
                <button type="button" class="mini-btn" onclick={() => (showKey = !showKey)}>
                  {showKey ? '隐藏' : '显示'}
                </button>
              </div>
            </label>

            <div class="field">
              <span class="field-label">模型名称（Gemini 3.5+，可点选或手动输入）</span>
              <div class="preset-row">
                {#each MODEL_PRESETS as preset}
                  <button
                    type="button"
                    class="preset-btn"
                    class:active={model === preset}
                    onclick={() => (model = preset)}
                  >{preset}</button>
                {/each}
              </div>
              <input class="text-input" type="text" bind:value={model} placeholder="gemini-3.8-flash" />
            </div>

            <label class="field">
              <span class="field-label">自定义代理 Base URL（可选，留空走官方地址）</span>
              <input
                class="text-input"
                type="url"
                bind:value={baseUrl}
                placeholder="https://generativelanguage.googleapis.com"
              />
            </label>

            <div class="field">
              <span class="field-label">界面主题</span>
              <div class="preset-row">
                {#each [['system', '跟随系统'], ['light', '浅色'], ['dark', '深色']] as [val, lbl]}
                  <button
                    type="button"
                    class="preset-btn"
                    class:active={theme === val}
                    onclick={() => (theme = val)}
                  >{lbl}</button>
                {/each}
              </div>
            </div>

            <div class="gemini-actions">
              <button type="button" class="primary-btn" disabled={busy} onclick={handleSave}>
                保存设置
              </button>
              <button type="button" class="secondary-btn" disabled={testing || busy} onclick={handleTestConnection}>
                {testing ? '测试中…' : '测试连接'}
              </button>
            </div>

            {#if testOkMsg}
              <p class="msg-ok">{testOkMsg}</p>
            {/if}
            {#if testErrMsg}
              <p class="msg-err">{testErrMsg}</p>
            {/if}
          </section>

          {#if statusMsg}
            <p class="msg-ok">{statusMsg}</p>
          {/if}
          {#if errorMsg}
            <p class="msg-err">{errorMsg}</p>
          {/if}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(15, 17, 22, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 16px;
    z-index: 50;
  }

  .modal {
    width: 100%;
    max-width: 500px;
    max-height: 90vh;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.25);
  }

  .modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 18px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .modal-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  .close-btn {
    padding: 2px 6px;
    font-size: 14px;
    color: var(--muted);
  }

  .tab-bar {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
    padding: 10px 18px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
    flex-shrink: 0;
  }

  .tab-btn {
    padding: 7px 6px;
    border-radius: 8px;
    font-size: 13px;
    color: var(--muted);
    font-weight: 500;
  }

  .tab-btn.active {
    background: var(--accent);
    color: var(--accent-fg);
    font-weight: 600;
  }

  .modal-body {
    padding: 16px 18px calc(18px + env(safe-area-inset-bottom));
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .block {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .block + .block {
    padding-top: 14px;
    border-top: 1px solid var(--border);
  }

  .block-title {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }

  .row-between {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .field-label,
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--muted);
  }

  .key-row {
    display: flex;
    gap: 6px;
  }

  .text-input {
    width: 100%;
    padding: 7px 10px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--bg);
    font-size: 13px;
    outline: none;
  }

  .text-input:focus {
    border-color: var(--accent);
  }

  .pin-input {
    letter-spacing: 0.2em;
    font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
    font-weight: 600;
  }

  .mini-btn,
  .preset-btn,
  .secondary-btn {
    padding: 6px 10px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--bg);
    font-size: 12px;
    white-space: nowrap;
  }

  .secondary-btn:hover:not(:disabled),
  .mini-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }

  .preset-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .preset-btn.active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }

  .gemini-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }

  .primary-btn,
  .gemini-actions .secondary-btn {
    min-height: 36px;
    padding: 8px 14px;
    border-radius: 8px;
    font-size: 13px;
  }

  .primary-btn {
    background: var(--accent);
    color: var(--accent-fg);
    font-weight: 600;
  }

  .server-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--bg);
  }

  .pin-box {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--accent-soft);
  }

  .pin-label {
    font-size: 12px;
    color: var(--accent);
    font-weight: 600;
  }

  .pin-code {
    font-size: 22px;
    letter-spacing: 0.14em;
    font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
    color: var(--accent);
  }

  .addr-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .addr-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .addr-text {
    font-size: 13px;
    font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
    word-break: break-all;
  }

  .alt-addrs {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .alt-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .peer-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .peer-item {
    width: 100%;
    text-align: left;
    padding: 8px 10px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .peer-item.selected,
  .peer-item:hover {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .peer-main {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }

  .peer-tag {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--muted);
  }

  .peer-addr {
    font-size: 12px;
    color: var(--muted);
    font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  }

  .backup-actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .hidden-file {
    display: none;
  }

  .check-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: 12px;
    color: var(--muted);
    cursor: pointer;
  }

  .msg-ok {
    margin: 0;
    font-size: 12px;
    color: var(--success);
    word-break: break-word;
  }

  .msg-err {
    margin: 0;
    font-size: 12px;
    color: var(--danger);
    word-break: break-word;
  }
</style>
