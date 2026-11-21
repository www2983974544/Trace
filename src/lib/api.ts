import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AiReport,
  AppSettings,
  Entry,
  EntrySummary,
  ExportPreview,
  GeminiChunkPayload,
  GeminiTestResult,
  ImportResult,
  LanPeer,
  LanServerInfo,
  LanSyncOutcome,
  LanSyncReceivedEvent,
  PeriodType,
} from './types';

export const getTodayDate = () => invoke<string>('get_today_date');

export const getEntry = (date: string) => invoke<Entry | null>('get_entry', { date });

export const saveEntry = (date: string, content: string) =>
  invoke<Entry | null>('save_entry', { date, content });

export const getMonthRecordedDates = (year: number, month: number) =>
  invoke<string[]>('get_month_recorded_dates', { year, month });

export const getRecentEntries = (excludeDate: string | null, limit: number) =>
  invoke<EntrySummary[]>('get_recent_entries', { excludeDate, limit });

export const previewPeriodExport = (
  anchorDate: string,
  period: PeriodType,
  format: 'md' | 'txt',
) => invoke<ExportPreview>('preview_period_export', { anchorDate, period, format });

export const saveTextToFile = (defaultFilename: string, content: string) =>
  invoke<string | null>('save_text_to_file', { defaultFilename, content });

export const exportBackupFile = () => invoke<string | null>('export_backup_file');

export const getBackupJson = () => invoke<string>('get_backup_json');

export const importBackupData = (jsonContent: string, overwrite: boolean) =>
  invoke<ImportResult>('import_backup_data', { jsonContent, overwrite });

export const getSettings = () => invoke<AppSettings>('get_settings');

export const saveSettings = (settings: AppSettings) =>
  invoke<AppSettings>('save_settings', { settings });

export const testGeminiConnection = (apiKey: string, model: string, baseUrl: string) =>
  invoke<GeminiTestResult>('test_gemini_connection', { apiKey, model, baseUrl });

export const analyzePeriodWithGemini = (
  requestId: string,
  anchorDate: string,
  period: PeriodType,
  prompt: string,
) => invoke<AiReport>('analyze_period_with_gemini', { requestId, anchorDate, period, prompt });

export const listAiReports = (limit = 20) => invoke<AiReport[]>('list_ai_reports', { limit });

export const deleteAiReport = (id: number) => invoke<void>('delete_ai_report', { id });

export const onGeminiChunk = (
  requestId: string,
  onDelta: (delta: string) => void,
): Promise<UnlistenFn> =>
  listen<GeminiChunkPayload>('gemini-chunk', (event) => {
    if (event.payload.requestId === requestId) {
      onDelta(event.payload.delta);
    }
  });

export const startLanSyncServer = () => invoke<LanServerInfo>('start_lan_sync_server');

export const stopLanSyncServer = () => invoke<void>('stop_lan_sync_server');

export const getLanSyncStatus = () => invoke<LanServerInfo | null>('get_lan_sync_status');

export const discoverLanPeers = (timeoutMs = 1800) =>
  invoke<LanPeer[]>('discover_lan_peers', { timeoutMs });

export const syncWithLanPeer = (address: string, pin: string) =>
  invoke<LanSyncOutcome>('sync_with_lan_peer', { address, pin });

export const onLanSyncReceived = (
  onReceived: (payload: LanSyncReceivedEvent) => void,
): Promise<UnlistenFn> =>
  listen<LanSyncReceivedEvent>('lan-sync-received', (event) => {
    onReceived(event.payload);
  });
