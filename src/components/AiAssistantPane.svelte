<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    analyzePeriodWithGemini,
    deleteAiReport,
    listAiReports,
    onGeminiChunk,
    previewPeriodExport,
    saveTextToFile,
  } from '../lib/api';
  import { renderMarkdown } from '../lib/markdown';
  import { DEFAULT_PROMPTS, PERIOD_LABELS, formatTimestamp } from '../lib/prompts';
  import type { AiReport, ExportPreview, PeriodType } from '../lib/types';

  interface Props {
    anchorDate: string;
    period: PeriodType;
    hasApiKey: boolean;
    showBackButton?: boolean;
    onSelectPeriod: (p: PeriodType) => void;
    onBackToWrite?: () => void;
    onOpenSettings: () => void;
  }

  let {
    anchorDate,
    period,
    hasApiKey,
    showBackButton = false,
    onSelectPeriod,
    onBackToWrite,
    onOpenSettings,
  }: Props = $props();

  const PERIODS: PeriodType[] = ['day', 'week', 'month', 'year'];

  let exportFormat = $state<'md' | 'txt'>('md');
  let preview = $state<ExportPreview | null>(null);
  let promptText = $state('');
  let streaming = $state(false);
  let streamOutput = $state('');
  let errorMsg = $state('');
  let actionNotice = $state('');
  let reports = $state<AiReport[]>([]);
  let activeReport = $state<AiReport | null>(null);

  let previewSeq = 0;
  let unlistenChunk: (() => void) | null = null;

  let displayMarkdown = $derived(streaming ? streamOutput : (activeReport?.content ?? streamOutput));
  let renderedResult = $derived(renderMarkdown(displayMarkdown));

  $effect(() => {
    const curPeriod = period;
    promptText = DEFAULT_PROMPTS[curPeriod];
  });

  $effect(() => {
    const d = anchorDate;
    const p = period;
    const fmt = exportFormat;
    const seq = ++previewSeq;
    previewPeriodExport(d, p, fmt)
      .then((res) => {
        if (seq === previewSeq) {
          preview = res;
        }
      })
      .catch((e) => {
        if (seq === previewSeq) {
          errorMsg = String(e);
        }
      });
  });

  onMount(async () => {
    try {
      reports = await listAiReports(20);
      if (reports.length > 0) {
        activeReport = reports[0];
      }
    } catch (e) {
      errorMsg = String(e);
    }
  });

  onDestroy(() => {
    unlistenChunk?.();
  });

  async function handleExportFile() {
    if (!preview || preview.entryCount === 0) return;
    actionNotice = '';
    try {
      const savedPath = await saveTextToFile(preview.filename, preview.content);
      if (savedPath) {
        actionNotice = `已保存：${savedPath}`;
      }
    } catch (e) {
      errorMsg = String(e);
    }
  }

  async function handleCopyExport() {
    if (!preview || preview.entryCount === 0) return;
    try {
      await navigator.clipboard.writeText(preview.content);
      actionNotice = `已复制 ${preview.entryCount} 篇日记文本`;
    } catch (e) {
      errorMsg = String(e);
    }
  }

  async function handleAnalyze() {
    if (streaming) return;
    if (!hasApiKey) {
      onOpenSettings();
      return;
    }
    if (!preview || preview.entryCount === 0) {
      errorMsg = '当前周期内暂无日记可分析';
      return;
    }

    errorMsg = '';
    actionNotice = '';
    streaming = true;
    streamOutput = '';
    activeReport = null;

    const requestId = crypto.randomUUID();
    try {
      unlistenChunk?.();
      unlistenChunk = await onGeminiChunk(requestId, (delta) => {
        streamOutput += delta;
      });

      const saved = await analyzePeriodWithGemini(requestId, anchorDate, period, promptText);
      activeReport = saved;
      reports = [saved, ...reports.filter((r) => r.id !== saved.id)];
      streamOutput = saved.content;
    } catch (e) {
      errorMsg = String(e);
    } finally {
      unlistenChunk?.();
      unlistenChunk = null;
      streaming = false;
    }
  }

  async function handleDeleteReport(id: number) {
    try {
      await deleteAiReport(id);
      reports = reports.filter((r) => r.id !== id);
      if (activeReport?.id === id) {
        activeReport = reports[0] ?? null;
      }
    } catch (e) {
      errorMsg = String(e);
    }
  }

  async function copyResult() {
    if (!displayMarkdown.trim()) return;
    await navigator.clipboard.writeText(displayMarkdown);
    actionNotice = '已复制分析报告';
  }
</script>

<section class="ai-shell">
  <header class="ai-topbar">
    <div class="topbar-left">
      {#if showBackButton && onBackToWrite}
        <button type="button" class="pill-btn" onclick={onBackToWrite}>← 返回写作</button>
      {/if}
      <h1 class="ai-title">周期导出 &amp; Gemini 助手</h1>
    </div>

    <div class="period-switch">
      {#each PERIODS as p}
        <button
          type="button"
          class="switch-btn"
          class:active={period === p}
          onclick={() => onSelectPeriod(p)}
        >
          {PERIOD_LABELS[p]}
        </button>
      {/each}
    </div>
  </header>

  <div class="ai-scroll">
    <div class="ai-inner">
      <div class="stats-card">
        <div class="stats-main">
          {#if preview}
            <span class="range-badge">{preview.range.label}</span>
            <span class="range-dates">{preview.range.startDate} ~ {preview.range.endDate}</span>
            <span class="range-counts">
              实写 <b>{preview.entryCount}</b> 篇 · {preview.charCount} 字 · 约 {preview.estimatedTokens} Tokens
            </span>
          {:else}
            <span class="range-dates">正在统计周期数据…</span>
          {/if}
        </div>

        <div class="export-controls">
          <div class="fmt-toggle">
            <button
              type="button"
              class="fmt-btn"
              class:active={exportFormat === 'md'}
              onclick={() => (exportFormat = 'md')}
            >.md</button>
            <button
              type="button"
              class="fmt-btn"
              class:active={exportFormat === 'txt'}
              onclick={() => (exportFormat = 'txt')}
            >.txt</button>
          </div>
          <button
            type="button"
            class="pill-btn"
            disabled={!preview || preview.entryCount === 0}
            onclick={handleExportFile}
          >保存文件</button>
          <button
            type="button"
            class="pill-btn"
            disabled={!preview || preview.entryCount === 0}
            onclick={handleCopyExport}
          >一键复制</button>
        </div>
      </div>

      {#if actionNotice}
        <p class="notice-ok">{actionNotice}</p>
      {/if}
      {#if errorMsg}
        <p class="notice-err">{errorMsg}</p>
      {/if}

      <div class="prompt-card">
        <div class="prompt-head">
          <span class="section-label">分析提示词（将附带所选周期全部日记发送给 Gemini）</span>
          {#if !hasApiKey}
            <button type="button" class="cfg-link" onclick={onOpenSettings}>未配置 API Key · 去设置</button>
          {/if}
        </div>
        <textarea class="prompt-input" rows="3" bind:value={promptText}></textarea>
        <div class="prompt-actions">
          <button
            type="button"
            class="run-btn"
            disabled={streaming || !preview || preview.entryCount === 0}
            onclick={handleAnalyze}
          >
            {#if streaming}
              Gemini 正在流式生成…
            {:else if !hasApiKey}
              先配置 Gemini API Key
            {:else}
              开始 Gemini 分析
            {/if}
          </button>
        </div>
      </div>

      {#if displayMarkdown}
        <div class="result-card">
          <div class="result-head">
            <span class="section-label">
              {#if streaming}
                实时流式输出
              {:else if activeReport}
                {activeReport.periodLabel} · {activeReport.model} · {formatTimestamp(activeReport.createdAt)}
              {:else}
                分析结果
              {/if}
            </span>
            <button type="button" class="pill-btn" onclick={copyResult}>复制结果</button>
          </div>
          <article class="markdown-body">{@html renderedResult}</article>
        </div>
      {/if}

      {#if reports.length > 0}
        <div class="history-card">
          <div class="section-label">历史分析记录 ({reports.length})</div>
          <div class="history-list">
            {#each reports as r}
              <div class="history-row" class:selected={activeReport?.id === r.id}>
                <button
                  type="button"
                  class="history-select"
                  onclick={() => {
                    activeReport = r;
                    streamOutput = r.content;
                  }}
                >
                  <span class="history-label">{r.periodLabel}</span>
                  <span class="history-meta">{r.model} · {formatTimestamp(r.createdAt)}</span>
                </button>
                <button
                  type="button"
                  class="del-btn"
                  aria-label="删除报告"
                  onclick={() => handleDeleteReport(r.id)}
                >删除</button>
              </div>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </div>
</section>

<style>
  .ai-shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg);
  }

  .ai-topbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 12px 20px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .topbar-left {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .ai-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  .period-switch {
    display: flex;
    gap: 4px;
    padding: 3px;
    border-radius: 8px;
    background: var(--bg);
    border: 1px solid var(--border);
  }

  .switch-btn {
    padding: 4px 12px;
    border-radius: 6px;
    font-size: 12px;
    color: var(--muted);
  }

  .switch-btn.active {
    background: var(--accent);
    color: var(--accent-fg);
    font-weight: 600;
  }

  .ai-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px 20px;
  }

  .ai-inner {
    max-width: 780px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .stats-card,
  .prompt-card,
  .result-card,
  .history-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 14px;
    box-shadow: var(--shadow);
  }

  .stats-card {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  .stats-main {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }

  .range-badge {
    padding: 2px 8px;
    border-radius: 6px;
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
    font-size: 12px;
  }

  .range-dates,
  .range-counts {
    color: var(--muted);
    font-size: 12px;
  }

  .export-controls {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .fmt-toggle {
    display: flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }

  .fmt-btn {
    padding: 4px 8px;
    font-size: 12px;
    background: var(--bg);
    color: var(--muted);
  }

  .fmt-btn.active {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }

  .pill-btn {
    padding: 5px 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    font-size: 12px;
  }

  .pill-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }

  .prompt-head,
  .result-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 8px;
  }

  .section-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }

  .cfg-link {
    font-size: 12px;
    color: var(--accent);
    text-decoration: underline;
  }

  .prompt-input {
    width: 100%;
    padding: 10px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg);
    resize: vertical;
    outline: none;
    font-size: 13px;
    line-height: 1.5;
  }

  .prompt-input:focus {
    border-color: var(--accent);
  }

  .prompt-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 10px;
  }

  .run-btn {
    padding: 8px 16px;
    border-radius: 8px;
    background: var(--accent);
    color: var(--accent-fg);
    font-size: 13px;
    font-weight: 600;
  }

  .notice-ok {
    margin: 0;
    font-size: 12px;
    color: var(--success);
  }

  .notice-err {
    margin: 0;
    font-size: 12px;
    color: var(--danger);
  }

  .history-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }

  .history-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg);
  }

  .history-row.selected {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .history-select {
    flex: 1;
    text-align: left;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .history-label {
    font-size: 13px;
    font-weight: 600;
  }

  .history-meta {
    font-size: 12px;
    color: var(--muted);
  }

  .del-btn {
    font-size: 12px;
    color: var(--muted);
    padding: 2px 6px;
  }

  .del-btn:hover {
    color: var(--danger);
  }

  @media (max-width: 768px) {
    .ai-topbar {
      padding: calc(10px + env(safe-area-inset-top)) 14px 10px;
    }

    .ai-scroll {
      padding: 12px 12px 24px;
    }
  }
</style>
