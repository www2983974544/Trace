<script lang="ts">
  import { renderMarkdown } from '../lib/markdown';
  import { formatDateWithWeekday } from '../lib/prompts';

  interface Props {
    date: string;
    todayDate: string;
    content: string;
    saveState: 'saved' | 'saving' | 'dirty';
    showBackButton?: boolean;
    onInput: (value: string) => void;
    onFlush: () => void;
    onBack?: () => void;
  }

  let {
    date,
    todayDate,
    content,
    saveState,
    showBackButton = false,
    onInput,
    onFlush,
    onBack,
  }: Props = $props();

  let previewMode = $state(false);
  let textareaEl = $state<HTMLTextAreaElement | null>(null);

  let isToday = $derived(date === todayDate);
  let wordCount = $derived(content.replace(/\s+/g, '').length);
  let renderedHtml = $derived(previewMode ? renderMarkdown(content) : '');

  let statusText = $derived.by(() => {
    if (saveState === 'saving') return '保存中…';
    if (saveState === 'dirty') return '待保存';
    return '已保存';
  });

  export function focusEditor() {
    if (!previewMode) {
      textareaEl?.focus();
    }
  }
</script>

<section class="editor-shell">
  <header class="editor-topbar">
    <div class="topbar-left">
      {#if showBackButton && onBack}
        <button
          type="button"
          class="back-btn"
          onclick={() => {
            onFlush();
            onBack();
          }}
        >← 返回</button>
      {/if}
      <h1 class="date-heading">{formatDateWithWeekday(date)}</h1>
      {#if isToday}
        <span class="today-tag">今日</span>
      {/if}
    </div>

    <div class="topbar-right">
      <span class="meta-text">{wordCount} 字</span>
      <span class="meta-text" class:saving={saveState !== 'saved'}>{statusText}</span>
      <button
        type="button"
        class="mode-btn"
        class:active={previewMode}
        onclick={() => (previewMode = !previewMode)}
      >
        {previewMode ? '编辑' : '预览'}
      </button>
    </div>
  </header>

  <div class="canvas-scroll">
    <div class="canvas-inner">
      {#if previewMode}
        {#if content.trim()}
          <article class="markdown-body">{@html renderedHtml}</article>
        {:else}
          <p class="empty-preview">这一天还没有留下文字记录，切换到「编辑」即可开始写作。</p>
        {/if}
      {:else}
        <textarea
          bind:this={textareaEl}
          class="diary-textarea"
          value={content}
          placeholder={isToday ? '记录今天发生了什么…（支持纯文字或轻量 Markdown）' : `补写 ${date} 的记录…`}
          oninput={(e) => onInput((e.currentTarget as HTMLTextAreaElement).value)}
          onblur={onFlush}
        ></textarea>
      {/if}
    </div>
  </div>
</section>

<style>
  .editor-shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--surface);
  }

  .editor-topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 20px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .topbar-left,
  .topbar-right {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .date-heading {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  .today-tag {
    font-size: 12px;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 500;
  }

  .meta-text {
    font-size: 12px;
    color: var(--muted);
  }

  .meta-text.saving {
    color: var(--accent);
  }

  .mode-btn,
  .back-btn {
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    font-size: 12px;
  }

  .mode-btn.active {
    background: var(--accent);
    color: var(--accent-fg);
    border-color: var(--accent);
  }

  .canvas-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 20px;
  }

  .canvas-inner {
    max-width: 720px;
    margin: 0 auto;
    height: 100%;
    display: flex;
    flex-direction: column;
  }

  .diary-textarea {
    width: 100%;
    flex: 1;
    min-height: 360px;
    resize: none;
    border: none;
    outline: none;
    background: transparent;
    font-size: 15px;
    line-height: 1.8;
    color: var(--text);
  }

  .diary-textarea::placeholder {
    color: var(--muted);
  }

  .empty-preview {
    color: var(--muted);
    margin: 0;
  }

  @media (max-width: 768px) {
    .editor-topbar {
      padding: calc(10px + env(safe-area-inset-top)) 14px 10px;
    }

    .canvas-scroll {
      padding: 14px 16px calc(16px + env(safe-area-inset-bottom));
    }
  }
</style>
