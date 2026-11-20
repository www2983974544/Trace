<script lang="ts">
  interface Props {
    selectedDate: string;
    todayDate: string;
    recordedDates: string[];
    viewYear: number;
    viewMonth: number;
    onSelectDate: (date: string) => void;
    onChangeMonth: (year: number, month: number) => void;
    onJumpToday: () => void;
  }

  let {
    selectedDate,
    todayDate,
    recordedDates,
    viewYear,
    viewMonth,
    onSelectDate,
    onChangeMonth,
    onJumpToday,
  }: Props = $props();

  const WEEK_HEADERS = ['一', '二', '三', '四', '五', '六', '日'];

  let recordedSet = $derived(new Set(recordedDates));

  let cells = $derived.by(() => {
    const first = new Date(viewYear, viewMonth - 1, 1);
    const startOffset = (first.getDay() + 6) % 7; // Monday = 0
    const daysInMonth = new Date(viewYear, viewMonth, 0).getDate();
    const list: Array<{ day: number; date: string } | null> = [];
    for (let i = 0; i < startOffset; i++) list.push(null);
    for (let d = 1; d <= daysInMonth; d++) {
      const mStr = String(viewMonth).padStart(2, '0');
      const dStr = String(d).padStart(2, '0');
      list.push({ day: d, date: `${viewYear}-${mStr}-${dStr}` });
    }
    return list;
  });

  function stepMonth(delta: number) {
    let nextMonth = viewMonth + delta;
    let nextYear = viewYear;
    if (nextMonth < 1) {
      nextMonth = 12;
      nextYear -= 1;
    } else if (nextMonth > 12) {
      nextMonth = 1;
      nextYear += 1;
    }
    onChangeMonth(nextYear, nextMonth);
  }
</script>

<section class="cal-card" aria-label="月历打卡网格">
  <header class="cal-header">
    <div class="cal-title">{viewYear} 年 {String(viewMonth).padStart(2, '0')} 月</div>
    <div class="cal-actions">
      <button type="button" class="icon-btn" onclick={() => stepMonth(-1)} aria-label="上个月">‹</button>
      <button type="button" class="today-btn" onclick={onJumpToday}>今天</button>
      <button type="button" class="icon-btn" onclick={() => stepMonth(1)} aria-label="下个月">›</button>
    </div>
  </header>

  <div class="week-row">
    {#each WEEK_HEADERS as w}
      <span class="week-label">{w}</span>
    {/each}
  </div>

  <div class="day-grid">
    {#each cells as cell}
      {#if cell === null}
        <span class="day-empty"></span>
      {:else}
        {@const isSelected = cell.date === selectedDate}
        {@const isToday = cell.date === todayDate}
        {@const hasEntry = recordedSet.has(cell.date)}
        <button
          type="button"
          class="day-btn"
          class:selected={isSelected}
          class:today={isToday && !isSelected}
          onclick={() => onSelectDate(cell.date)}
          aria-label={cell.date}
          aria-pressed={isSelected}
        >
          <span class="day-num">{cell.day}</span>
          <span class="day-dot" class:visible={hasEntry}></span>
        </button>
      {/if}
    {/each}
  </div>
</section>

<style>
  .cal-card {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 12px;
    border-radius: 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
  }

  .cal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }

  .cal-title {
    font-size: 14px;
    font-weight: 600;
  }

  .cal-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .icon-btn,
  .today-btn {
    height: 26px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    font-size: 12px;
    padding: 0 8px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .icon-btn:hover,
  .today-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .week-row {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 3px;
    margin-bottom: 4px;
  }

  .week-label {
    text-align: center;
    font-size: 12px;
    color: var(--muted);
  }

  .day-grid {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 3px;
    flex: 1;
    align-content: start;
  }

  .day-empty {
    height: 32px;
  }

  .day-btn {
    height: 32px;
    border-radius: 7px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    padding: 2px 0;
    transition: background-color 120ms ease, border-color 120ms ease;
  }

  .day-btn:hover {
    background: var(--accent-soft);
  }

  .day-btn.today {
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }

  .day-btn.selected {
    background: var(--accent);
    color: var(--accent-fg);
    font-weight: 600;
  }

  .day-num {
    font-size: 12px;
    line-height: 1.1;
  }

  .day-dot {
    width: 4px;
    height: 4px;
    border-radius: 999px;
    background: transparent;
  }

  .day-dot.visible {
    background: var(--dot);
  }

  .day-btn.selected .day-dot.visible {
    background: var(--accent-fg);
  }
</style>
