export type ProviderId = 'claude' | 'codex';
export interface UsageWindow { id: string; label: string; usedPercent: number | null; resetsAt: number | null; durationMins: number | null }
export interface Snapshot { provider: ProviderId; state: string; accountId: string | null; accountLabel: string | null; windows: UsageWindow[]; observedAt: number | null; source: string; message: string | null }
export interface Settings { theme: 'system' | 'light' | 'dark'; showUsed: boolean; alerts: boolean; codexEnabled: boolean; claudeEnabled: boolean }
export interface View { settings: Settings; providers: Snapshot[]; autostart: boolean }
