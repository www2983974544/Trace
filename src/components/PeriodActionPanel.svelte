<script lang="ts">
  import { previewPeriodExport, saveTextToFile } from '../lib/api';
  import { PERIOD_LABELS } from '../lib/prompts';
  import { APP_VERSION_LABEL } from '../lib/version';
  import type { PeriodType } from '../lib/types';

  interface Props {
    anchorDate: string;
    selectedPeriod: PeriodType;
    onSelectPeriod: (p: PeriodType) => void;
    onOpenAiPane: () => void;
    onOpenSync: () => void;
    onOpenSettings: () => void;
    onOpenChangelog: () => void;
  }

  let {
    anchorDate,
    selectedPeriod,
    onSelectPeriod,
    onOpenAiPane,
    onOpenSync,
    onOpenSettings,
    onOpenChangelog,
  }: Props = $props();

  const PERIODS: PeriodType[] = ['day', 'week', 'month', 'year'];
  let feedback = $state('');
  let busy = $state(false);

  async function quickExport(mode: 'md' | 'txt' | 'copy') {
    if (busy) return;
    busy = true;
    feedback = '';
    try {
      const preview = await previewPeriodExport(anchorDate, selectedPeriod, mode === 'txt' ? 'txt' : 'md');
      if (preview.entryCount === 0) {
        feedback = '该周期暂无日记可导出';
        return;
      }
      if (mode === 'copy') {
        await navigator.clipboard.writeText(preview.content);
        feedback = `已复制 ${preview.entryCount} 篇 (${preview.range.label})`;
      } else {
        const savedPath = await saveTextToFile(preview.filename, preview.content);
        if (savedPath) {
          feedback = `已导出 ${preview.filename}`;
        }
      }
    } catch (e) {
      feedback = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="panel-card" aria-label="快捷导出与 AI 助手">
  <div class="panel-top">
    <span class="panel-title">周期导出 &amp; AI</span>
    <span class="anchor-tag">锚点 {anchorDate.slice(5)}</span>
  </div>

  <div class="period-tabs" role="tablist" aria-label="时间粒度">
    {#each PERIODS as p}
      <button
        type="button"
        class="period-btn"
        class:active={selectedPeriod === p}
        onclick={() => onSelectPeriod(p)}
      >
        {PERIOD_LABELS[p]}
      </button>
    {/each}
  </div>

  <div class="export-row">
    <button type="button" class="sub-btn" disabled={busy} onclick={() => quickExport('md')}>导出 .md</button>
    <button type="button" class="sub-btn" disabled={busy} onclick={() => quickExport('txt')}>导出 .txt</button>
    <button type="button" class="sub-btn" disabled={busy} onclick={() => quickExport('copy')}>复制</button>
  </div>

  <button type="button" class="ai-primary-btn" onclick={onOpenAiPane}>
    Gemini AI 助手
  </button>

  {#if feedback}
    <p class="feedback-line">{feedback}</p>
  {/if}

  <div class="panel-footer">
    <button type="button" class="link-btn" onclick={onOpenSync}>Wi-Fi 快传</button>
    <button type="button" class="link-btn" onclick={onOpenSettings}>备份 / 设置</button>
    <button
      type="button"
      class="ver-btn"
      title="查看版本更新日志"
      onclick={onOpenChangelog}
    >{APP_VERSION_LABEL}</button>
  </div>
</section>

<style>
  .panel-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border-radius: 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
  }

  .panel-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .panel-title {
    font-size: 13px;
    font-weight: 600;
  }

  .anchor-tag {
    font-size: 12px;
    color: var(--muted);
  }

  .period-tabs {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 4px;
    padding: 3px;
    border-radius: 8px;
    background: var(--bg);
    border: 1px solid var(--border);
  }

  .period-btn {
    padding: 5px 0;
    border-radius: 6px;
    font-size: 13px;
    color: var(--muted);
  }

  .period-btn.active {
    background: var(--accent);
    color: var(--accent-fg);
    font-weight: 600;
  }

  .export-row {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
  }

  .sub-btn {
    padding: 6px 4px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--bg);
    font-size: 12px;
  }

  .sub-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }

  .ai-primary-btn {
    width: 100%;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--accent);
    color: var(--accent-fg);
    font-size: 13px;
    font-weight: 600;
  }

  .ai-primary-btn:hover {
    opacity: 0.92;
  }

  .feedback-line {
    margin: 0;
    font-size: 12px;
    color: var(--accent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .panel-footer {
    margin-top: auto;
    padding-top: 6px;
    border-top: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .link-btn {
    font-size: 12px;
    color: var(--muted);
    padding: 2px 0;
  }

  .link-btn:hover {
    color: var(--accent);
  }

  .ver-btn {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--muted);
    font-weight: 600;
  }

  .ver-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
</style>
