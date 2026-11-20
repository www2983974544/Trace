export type PeriodType = 'day' | 'week' | 'month' | 'year';

export interface Entry {
  id: number;
  entryDate: string;
  content: string;
  wordCount: number;
  createdAt: number;
  updatedAt: number;
}

export interface EntrySummary {
  id: number;
  entryDate: string;
  preview: string;
  wordCount: number;
  updatedAt: number;
}

export interface AiReport {
  id: number;
  periodType: PeriodType | string;
  periodLabel: string;
  startDate: string;
  endDate: string;
  model: string;
  prompt: string;
  content: string;
  createdAt: number;
}

export interface AppSettings {
  geminiApiKey: string;
  geminiModel: string;
  geminiBaseUrl: string;
  theme: 'system' | 'light' | 'dark' | string;
}

export interface PeriodRange {
  periodType: PeriodType;
  label: string;
  startDate: string;
  endDate: string;
}

export interface ExportPreview {
  range: PeriodRange;
  entryCount: number;
  charCount: number;
  estimatedTokens: number;
  filename: string;
  content: string;
}

export interface ImportResult {
  importedEntries: number;
  updatedEntries: number;
  skippedEntries: number;
  importedReports: number;
}

export interface GeminiChunkPayload {
  requestId: string;
  delta: string;
}

export interface GeminiTestResult {
  model: string;
  latencyMs: number;
  replyPreview: string;
}
