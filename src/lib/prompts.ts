import type { PeriodType } from './types';

export const PERIOD_LABELS: Record<PeriodType, string> = {
  day: '日',
  week: '周',
  month: '月',
  year: '年',
};

export const DEFAULT_PROMPTS: Record<PeriodType, string> = {
  day: '请梳理今天的日常记录，提炼关键事件与值得延续的小收获。',
  week: '请根据本周日记进行周度复盘：1. 核心事项回顾；2. 精力分配与状态变化；3. 下周一条最具体的改进建议。',
  month: '请结合本月日记做一次月度洞察：总结本月主线成长、反复出现的卡点，以及下月值得保留的习惯。',
  year: '请通读今年全部日记，梳理这一年的关键转折、个人成长主线、关注点变化，并给下一年的我写一段真诚建议。',
};

const WEEKDAYS = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];

export function parseYmd(dateStr: string): { year: number; month: number; day: number } {
  const [y, m, d] = dateStr.split('-').map(Number);
  return { year: y, month: m, day: d };
}

export function formatWeekdayZh(dateStr: string): string {
  const { year, month, day } = parseYmd(dateStr);
  const dt = new Date(year, month - 1, day);
  return WEEKDAYS[dt.getDay()] ?? '';
}

export function formatDateWithWeekday(dateStr: string): string {
  return `${dateStr} ${formatWeekdayZh(dateStr)}`;
}

export function formatTimestamp(unixSec: number): string {
  const d = new Date(unixSec * 1000);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
