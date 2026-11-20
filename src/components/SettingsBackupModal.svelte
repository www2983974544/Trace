<script lang="ts">
  import { testGeminiConnection } from '../lib/api';
  import type { AppSettings, ImportResult } from '../lib/types';

  interface Props {
    open: boolean;
    settings: AppSettings;
    onClose: () => void;
    onSaveSettings: (next: AppSettings) => Promise<void>;
    onExportBackup: () => Promise<string | null>;
    onImportBackup: (jsonContent: string, overwrite: boolean) => Promise<ImportResult>;
  }

  let {
    open,
    settings,
    onClose,
    onSaveSettings,
    onExportBackup,
    onImportBackup,
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

  $effect(() => {
    if (open) {
      apiKey = settings.geminiApiKey;
      model = settings.geminiModel || 'gemini-3.8-flash';
      baseUrl = settings.geminiBaseUrl;
      theme = settings.theme || 'system';
      testOkMsg = '';
      testErrMsg = '';
      statusMsg = '';
      errorMsg = '';
    }
  });

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
      testOkMsg = `模型 ${res.model} 可用（耗时 ${res.latencyMs} ms${preview}）`;
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
      aria-label="Gemini 与备份设置"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
    >
      <header class="modal-head">
        <h2 class="modal-title">设置与本地备份</h2>
        <button type="button" class="close-btn" onclick={onClose} aria-label="关闭">✕</button>
      </header>

      <div class="modal-body">
        <section class="block">
          <h3 class="block-title">Gemini 大模型接口</h3>

          <label class="field">
            <span class="field-label">API Key（仅存本地私有数据库）</span>
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

        <section class="block">
          <h3 class="block-title">本地数据备份与恢复（.json）</h3>
          <p class="hint">单文件包含全部日记正文与历史 Gemini 分析报告，可在 Windows 与 Android 间手动迁移。</p>

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
    max-width: 480px;
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

  .modal-body {
    padding: 16px 18px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 18px;
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

  .secondary-btn:hover:not(:disabled) {
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
    word-break: break-all;
  }

  .msg-err {
    margin: 0;
    font-size: 12px;
    color: var(--danger);
    word-break: break-word;
  }
</style>
