<script lang="ts">
  import { onMount } from 'svelte';
  import AiAssistantPane from './components/AiAssistantPane.svelte';
  import CalendarGrid from './components/CalendarGrid.svelte';
  import ImmersiveEditor from './components/ImmersiveEditor.svelte';
  import PeriodActionPanel from './components/PeriodActionPanel.svelte';
  import SettingsBackupModal from './components/SettingsBackupModal.svelte';
  import {
    exportBackupFile,
    getEntry,
    getMonthRecordedDates,
    getRecentEntries,
    getSettings,
    getTodayDate,
    importBackupData,
    saveEntry,
    saveSettings,
  } from './lib/api';
  import { formatDateWithWeekday, parseYmd } from './lib/prompts';
  import type { AppSettings, Entry, EntrySummary, ImportResult, PeriodType } from './lib/types';

  let todayDate = $state('');
  let selectedDate = $state('');
  let loadedDate = $state('');
  let viewYear = $state(new Date().getFullYear());
  let viewMonth = $state(new Date().getMonth() + 1);

  let editorContent = $state('');
  let saveState = $state<'saved' | 'saving' | 'dirty'>('saved');
  let todayPreview = $state<Entry | null>(null);
  let recordedDates = $state<string[]>([]);
  let recentEntries = $state<EntrySummary[]>([]);

  let selectedPeriod = $state<PeriodType>('week');
  let desktopRightMode = $state<'editor' | 'ai'>('editor');
  let mobileTab = $state<'today' | 'calendar' | 'ai'>('today');
  let mobileEditing = $state(false);

  let settings = $state<AppSettings>({
    geminiApiKey: '',
    geminiModel: 'gemini-3.8-flash',
    geminiBaseUrl: '',
    theme: 'system',
  });
  let modalOpen = $state(false);
  let ready = $state(false);
  let initError = $state('');

  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let editorRef = $state<{ focusEditor?: () => void } | null>(null);

  function applyTheme(theme: string) {
    const root = document.documentElement;
    if (theme === 'light' || theme === 'dark') {
      root.dataset.theme = theme;
    } else {
      delete root.dataset.theme;
    }
  }

  async function refreshSideData(year = viewYear, month = viewMonth) {
    const [dates, recent] = await Promise.all([
      getMonthRecordedDates(year, month),
      getRecentEntries(todayDate || null, 15),
    ]);
    recordedDates = dates;
    recentEntries = recent;
  }

  async function flushSave() {
    if (debounceTimer) {
      clearTimeout(debounceTimer);
      debounceTimer = null;
    }
    if (!loadedDate || saveState === 'saved') return;

    const targetDate = loadedDate;
    const contentToSave = editorContent;
    saveState = 'saving';
    try {
      const saved = await saveEntry(targetDate, contentToSave);
      if (loadedDate === targetDate && editorContent === contentToSave) {
        saveState = 'saved';
      }
      if (targetDate === todayDate) {
        todayPreview = saved;
      }
      await refreshSideData();
    } catch {
      saveState = 'dirty';
    }
  }

  function handleEditorInput(val: string) {
    editorContent = val;
    saveState = 'dirty';
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      void flushSave();
    }, 500);
  }

  async function selectDate(date: string, openMobileEdit = false) {
    if (date !== loadedDate) {
      await flushSave();
      const { year, month } = parseYmd(date);
      const monthChanged = year !== viewYear || month !== viewMonth;
      viewYear = year;
      viewMonth = month;
      selectedDate = date;
      const entry = await getEntry(date);
      loadedDate = date;
      editorContent = entry?.content ?? '';
      saveState = 'saved';
      if (date === todayDate) {
        todayPreview = entry;
      }
      if (monthChanged) {
        recordedDates = await getMonthRecordedDates(year, month);
      }
    }
    desktopRightMode = 'editor';
    if (openMobileEdit) {
      mobileEditing = true;
    }
  }

  async function handleChangeMonth(year: number, month: number) {
    viewYear = year;
    viewMonth = month;
    recordedDates = await getMonthRecordedDates(year, month);
  }

  async function handleJumpToday() {
    const freshToday = await getTodayDate();
    todayDate = freshToday;
    await selectDate(freshToday, false);
  }

  async function openTodayMobileEditor() {
    const freshToday = await getTodayDate();
    todayDate = freshToday;
    await selectDate(freshToday, true);
    queueMicrotask(() => editorRef?.focusEditor?.());
  }

  async function closeMobileEditor() {
    await flushSave();
    mobileEditing = false;
  }

  async function handleSaveSettings(next: AppSettings) {
    const saved = await saveSettings(next);
    settings = saved;
    applyTheme(saved.theme);
  }

  async function handleImportBackup(jsonContent: string, overwrite: boolean): Promise<ImportResult> {
    await flushSave();
    const res = await importBackupData(jsonContent, overwrite);
    if (loadedDate) {
      const refreshed = await getEntry(loadedDate);
      editorContent = refreshed?.content ?? '';
      saveState = 'saved';
      if (loadedDate === todayDate) {
        todayPreview = refreshed;
      }
    }
    if (todayDate && todayDate !== loadedDate) {
      todayPreview = await getEntry(todayDate);
    }
    await refreshSideData();
    return res;
  }

  onMount(async () => {
    try {
      const [today, loadedSettings] = await Promise.all([getTodayDate(), getSettings()]);
      todayDate = today;
      selectedDate = today;
      loadedDate = today;
      settings = loadedSettings;
      applyTheme(loadedSettings.theme);

      const { year, month } = parseYmd(today);
      viewYear = year;
      viewMonth = month;

      const [todayEntry, dates, recent] = await Promise.all([
        getEntry(today),
        getMonthRecordedDates(year, month),
        getRecentEntries(today, 15),
      ]);
      todayPreview = todayEntry;
      editorContent = todayEntry?.content ?? '';
      recordedDates = dates;
      recentEntries = recent;
      ready = true;
    } catch (e) {
      initError = String(e);
    }
  });

  function handleWindowHide() {
    void flushSave();
  }
</script>

<svelte:window onbeforeunload={handleWindowHide} onpagehide={handleWindowHide} />

{#if initError}
  <main class="boot-state">
    <p>启动失败：{initError}</p>
  </main>
{:else if !ready}
  <main class="boot-state">
    <p>正在打开今日记录…</p>
  </main>
{:else}
  <!-- Windows Desktop Layout (> 768px) -->
  <div class="desktop-layout">
    <aside class="desktop-sidebar">
      <div class="sidebar-top">
        <CalendarGrid
          {selectedDate}
          {todayDate}
          {recordedDates}
          {viewYear}
          {viewMonth}
          onSelectDate={(d) => void selectDate(d, false)}
          onChangeMonth={(y, m) => void handleChangeMonth(y, m)}
          onJumpToday={() => void handleJumpToday()}
        />
      </div>
      <div class="sidebar-bottom">
        <PeriodActionPanel
          anchorDate={selectedDate}
          {selectedPeriod}
          onSelectPeriod={(p) => (selectedPeriod = p)}
          onOpenAiPane={() => (desktopRightMode = 'ai')}
          onOpenSettings={() => (modalOpen = true)}
        />
      </div>
    </aside>

    <main class="desktop-main">
      {#if desktopRightMode === 'ai'}
        <AiAssistantPane
          anchorDate={selectedDate}
          period={selectedPeriod}
          hasApiKey={Boolean(settings.geminiApiKey.trim())}
          showBackButton={true}
          onSelectPeriod={(p) => (selectedPeriod = p)}
          onBackToWrite={() => (desktopRightMode = 'editor')}
          onOpenSettings={() => (modalOpen = true)}
        />
      {:else}
        <ImmersiveEditor
          bind:this={editorRef}
          date={selectedDate}
          {todayDate}
          content={editorContent}
          {saveState}
          onInput={handleEditorInput}
          onFlush={() => void flushSave()}
        />
      {/if}
    </main>
  </div>

  <!-- Android Mobile Layout (<= 768px, Scheme A with high-weight Today Hero Card) -->
  <div class="mobile-layout">
    {#if mobileEditing}
      <div class="mobile-fullscreen">
        <ImmersiveEditor
          bind:this={editorRef}
          date={selectedDate}
          {todayDate}
          content={editorContent}
          {saveState}
          showBackButton={true}
          onInput={handleEditorInput}
          onFlush={() => void flushSave()}
          onBack={() => void closeMobileEditor()}
        />
      </div>
    {:else}
      <div class="mobile-content">
        {#if mobileTab === 'today'}
          <header class="mob-topbar">
            <span class="mob-brand">{todayDate.slice(0, 7)} · Trace</span>
            <button type="button" class="mob-util-btn" onclick={() => (modalOpen = true)}>
              备份 / 设置
            </button>
          </header>

          <button type="button" class="today-hero-card" onclick={() => void openTodayMobileEditor()}>
            <div class="hero-head">
              <div>
                <div class="hero-kicker">今日记录 · 默认当天</div>
                <div class="hero-date">{formatDateWithWeekday(todayDate)}</div>
              </div>
              <span class="hero-pill">
                {todayPreview ? `${todayPreview.wordCount} 字 · 续写` : '点击开始写'}
              </span>
            </div>

            <div class="hero-body">
              {#if todayPreview && todayPreview.content.trim()}
                <p class="hero-preview">{todayPreview.content}</p>
              {:else}
                <p class="hero-placeholder">
                  今天还没有写下文字。<br />
                  点击这张卡片，立即进入无干扰全屏写作画布…
                </p>
              {/if}
            </div>

            <div class="hero-foot">
              <span>轻触卡片编辑今天</span>
              <span>纯文字 / 轻量 Markdown →</span>
            </div>
          </button>

          <section class="recent-section" aria-label="近期历史记录">
            <div class="recent-title">近期记录</div>
            {#if recentEntries.length === 0}
              <p class="recent-empty">暂无历史记录，从今天开始积累第一篇吧。</p>
            {:else}
              <div class="recent-list">
                {#each recentEntries as item}
                  <button
                    type="button"
                    class="recent-item"
                    onclick={() => void selectDate(item.entryDate, true)}
                  >
                    <div class="recent-item-head">
                      <span class="recent-date">{formatDateWithWeekday(item.entryDate)}</span>
                      <span class="recent-words">{item.wordCount} 字</span>
                    </div>
                    <p class="recent-snippet">{item.preview}</p>
                  </button>
                {/each}
              </div>
            {/if}
          </section>
        {:else if mobileTab === 'calendar'}
          <div class="mob-calendar-page">
            <div class="mob-cal-wrap">
              <CalendarGrid
                {selectedDate}
                {todayDate}
                {recordedDates}
                {viewYear}
                {viewMonth}
                onSelectDate={(d) => void selectDate(d, false)}
                onChangeMonth={(y, m) => void handleChangeMonth(y, m)}
                onJumpToday={() => void handleJumpToday()}
              />
            </div>

            <button
              type="button"
              class="mob-selected-card"
              onclick={() => {
                mobileEditing = true;
              }}
            >
              <div class="recent-item-head">
                <span class="recent-date">{formatDateWithWeekday(selectedDate)}</span>
                <span class="hero-cta">{editorContent.trim() ? '点击编辑 →' : '补录这一天 →'}</span>
              </div>
              <p class="mob-selected-preview">
                {editorContent.trim() ? editorContent : '这一天暂无内容，点击即可补写。'}
              </p>
            </button>
          </div>
        {:else}
          <AiAssistantPane
            anchorDate={selectedDate}
            period={selectedPeriod}
            hasApiKey={Boolean(settings.geminiApiKey.trim())}
            onSelectPeriod={(p) => (selectedPeriod = p)}
            onOpenSettings={() => (modalOpen = true)}
          />
        {/if}
      </div>

      <nav class="mob-bottom-nav" aria-label="主导航">
        <button
          type="button"
          class="nav-btn"
          class:active={mobileTab === 'today'}
          onclick={() => (mobileTab = 'today')}
        >今日</button>
        <button
          type="button"
          class="nav-btn"
          class:active={mobileTab === 'calendar'}
          onclick={() => (mobileTab = 'calendar')}
        >日历</button>
        <button
          type="button"
          class="nav-btn"
          class:active={mobileTab === 'ai'}
          onclick={() => (mobileTab = 'ai')}
        >导出 &amp; AI</button>
      </nav>
    {/if}
  </div>

  <SettingsBackupModal
    open={modalOpen}
    {settings}
    onClose={() => (modalOpen = false)}
    onSaveSettings={handleSaveSettings}
    onExportBackup={exportBackupFile}
    onImportBackup={handleImportBackup}
  />
{/if}

<style>
  .boot-state {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--muted);
  }

  .desktop-layout {
    display: flex;
    height: 100%;
    width: 100%;
    overflow: hidden;
  }

  .desktop-sidebar {
    width: 32%;
    min-width: 268px;
    max-width: 348px;
    height: 100%;
    padding: 12px;
    background: var(--sidebar);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow-y: auto;
  }

  .sidebar-top {
    flex: 56 1 0%;
    min-height: 256px;
  }

  .sidebar-bottom {
    flex: 44 1 0%;
    display: flex;
    flex-direction: column;
  }

  .sidebar-bottom :global(.panel-card) {
    height: 100%;
  }

  .desktop-main {
    flex: 1;
    min-width: 0;
    height: 100%;
  }

  .mobile-layout {
    display: none;
  }

  @media (max-width: 768px) {
    .desktop-layout {
      display: none;
    }

    .mobile-layout {
      display: flex;
      flex-direction: column;
      height: 100%;
      width: 100%;
      overflow: hidden;
      background: var(--bg);
    }

    .mobile-fullscreen {
      flex: 1;
      min-height: 0;
    }

    .mobile-content {
      flex: 1;
      min-height: 0;
      display: flex;
      flex-direction: column;
      overflow: hidden;
    }

    .mob-topbar {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: calc(10px + env(safe-area-inset-top)) 14px 8px;
      flex-shrink: 0;
    }

    .mob-brand {
      font-size: 15px;
      font-weight: 600;
    }

    .mob-util-btn {
      font-size: 12px;
      padding: 5px 10px;
      border-radius: 999px;
      border: 1px solid var(--border);
      background: var(--surface);
      color: var(--muted);
    }

    /* Scheme A enhanced: Today Hero Card occupies ~54% of content height */
    .today-hero-card {
      flex: 54 1 0%;
      min-height: 220px;
      margin: 4px 14px 10px;
      padding: 18px;
      border-radius: 16px;
      background: var(--accent);
      color: var(--accent-fg);
      text-align: left;
      display: flex;
      flex-direction: column;
      justify-content: space-between;
      box-shadow: 0 8px 20px rgba(59, 91, 219, 0.22);
    }

    .hero-head {
      display: flex;
      align-items: flex-start;
      justify-content: space-between;
      gap: 8px;
    }

    .hero-kicker {
      font-size: 12px;
      opacity: 0.86;
    }

    .hero-date {
      font-size: 20px;
      font-weight: 700;
      margin-top: 2px;
    }

    .hero-pill {
      font-size: 12px;
      padding: 4px 10px;
      border-radius: 999px;
      background: rgba(255, 255, 255, 0.2);
      white-space: nowrap;
    }

    .hero-body {
      flex: 1;
      min-height: 0;
      display: flex;
      align-items: center;
      padding: 10px 0;
      overflow: hidden;
    }

    .hero-preview {
      margin: 0;
      font-size: 15px;
      line-height: 1.65;
      display: -webkit-box;
      -webkit-line-clamp: 5;
      line-clamp: 5;
      -webkit-box-orient: vertical;
      overflow: hidden;
      white-space: pre-wrap;
    }

    .hero-placeholder {
      margin: 0;
      font-size: 15px;
      line-height: 1.65;
      opacity: 0.9;
    }

    .hero-foot {
      display: flex;
      align-items: center;
      justify-content: space-between;
      font-size: 12px;
      opacity: 0.9;
      padding-top: 8px;
      border-top: 1px solid rgba(255, 255, 255, 0.22);
    }

    .recent-section {
      flex: 36 1 0%;
      min-height: 0;
      display: flex;
      flex-direction: column;
      padding: 0 14px 10px;
    }

    .recent-title {
      font-size: 12px;
      font-weight: 600;
      color: var(--muted);
      margin-bottom: 6px;
      flex-shrink: 0;
    }

    .recent-empty {
      margin: 0;
      font-size: 13px;
      color: var(--muted);
    }

    .recent-list {
      flex: 1;
      min-height: 0;
      overflow-y: auto;
      display: flex;
      flex-direction: column;
      gap: 6px;
    }

    .recent-item {
      width: 100%;
      text-align: left;
      padding: 10px 12px;
      border-radius: 10px;
      border: 1px solid var(--border);
      background: var(--surface);
    }

    .recent-item-head {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 3px;
    }

    .recent-date {
      font-size: 13px;
      font-weight: 600;
    }

    .recent-words {
      font-size: 12px;
      color: var(--muted);
    }

    .recent-snippet {
      margin: 0;
      font-size: 13px;
      color: var(--muted);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .mob-calendar-page {
      flex: 1;
      min-height: 0;
      overflow-y: auto;
      padding: calc(12px + env(safe-area-inset-top)) 14px 14px;
      display: flex;
      flex-direction: column;
      gap: 12px;
    }

    .mob-cal-wrap {
      flex-shrink: 0;
    }

    .mob-selected-card {
      text-align: left;
      padding: 14px;
      border-radius: 12px;
      border: 1px solid var(--border);
      background: var(--surface);
    }

    .hero-cta {
      font-size: 12px;
      color: var(--accent);
      font-weight: 600;
    }

    .mob-selected-preview {
      margin: 6px 0 0;
      font-size: 13px;
      color: var(--muted);
      display: -webkit-box;
      -webkit-line-clamp: 4;
      line-clamp: 4;
      -webkit-box-orient: vertical;
      overflow: hidden;
      white-space: pre-wrap;
    }

    .mob-bottom-nav {
      display: grid;
      grid-template-columns: repeat(3, 1fr);
      border-top: 1px solid var(--border);
      background: var(--surface);
      padding-bottom: env(safe-area-inset-bottom);
      flex-shrink: 0;
    }

    .nav-btn {
      padding: 12px 0;
      font-size: 13px;
      color: var(--muted);
      font-weight: 500;
    }

    .nav-btn.active {
      color: var(--accent);
      font-weight: 700;
    }
  }
</style>
