<script lang="ts">
  import { renderMarkdown } from '../lib/markdown';
  import { APP_VERSION_LABEL, CHANGELOG_MARKDOWN } from '../lib/version';

  interface Props {
    open: boolean;
    onClose: () => void;
  }

  let { open, onClose }: Props = $props();

  let renderedHtml = $derived(renderMarkdown(CHANGELOG_MARKDOWN));

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
      aria-label="版本更新日志"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
    >
      <header class="modal-head">
        <div class="title-wrap">
          <h2 class="modal-title">版本更新日志</h2>
          <span class="ver-badge">{APP_VERSION_LABEL}</span>
        </div>
        <button type="button" class="close-btn" onclick={onClose} aria-label="关闭">✕</button>
      </header>

      <div class="modal-body">
        <article class="markdown-body">{@html renderedHtml}</article>
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
    z-index: 60;
  }

  .modal {
    width: 100%;
    max-width: 540px;
    max-height: 86vh;
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

  .title-wrap {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .modal-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  .ver-badge {
    font-size: 12px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }

  .close-btn {
    padding: 2px 6px;
    font-size: 14px;
    color: var(--muted);
  }

  .modal-body {
    padding: 12px 18px calc(18px + env(safe-area-inset-bottom));
    overflow-y: auto;
  }
</style>
