import { useCallback, useEffect, useState } from 'react';
import { Bell, Check, Clock3, Gauge, Laptop, Moon, Plug, Power, RefreshCw, Settings2, Sun, X } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { isPermissionGranted, requestPermission } from '@tauri-apps/plugin-notification';
import { call, desktop } from './bridge';
import { age, countdown, freshness, remaining, windowState } from './domain';
import type { ProviderId, Settings, Snapshot, View } from './types';

const names = { claude: 'Claude Code', codex: 'Codex' };

function ProviderMark({ id }: { id: ProviderId }) {
  return (
    <span className={`provider-mark ${id}`} aria-hidden="true">
      {id === 'claude' ? <span>✳</span> : <svg viewBox="0 0 24 24" fill="none"><path d="M9 5 3 12l6 7M15 5l6 7-6 7M14 3l-4 18" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" /></svg>}
    </span>
  );
}

function ProviderCard({ snapshot, showUsed, now, busy, onConnect }: { snapshot: Snapshot; showUsed: boolean; now: number; busy: boolean; onConnect: (id: ProviderId, enabled: boolean) => void }) {
  const fresh = freshness(snapshot, now);
  const connected = snapshot.state === 'ready';
  const stateLabel = connected
    ? fresh === 'fresh' ? 'Live' : 'Stale'
    : ({ disconnected: 'Not connected', missing_cli: 'CLI not found', signed_out: 'Signed out', waiting: 'Waiting', error: 'Error', unsupported: 'Unsupported', unavailable: 'Unavailable' }[snapshot.state] ?? snapshot.state);

  return (
    <article className={`provider-card ${snapshot.provider}`}>
      <header className="provider-header">
        <ProviderMark id={snapshot.provider} />
        <div className="provider-title">
          <h2>{names[snapshot.provider]}</h2>
          <span className="account" title={snapshot.accountLabel ?? snapshot.source}>{snapshot.accountLabel ?? stateLabel}</span>
        </div>
        <span className={`status-pill ${connected && fresh === 'fresh' ? 'live' : ''}`}>{stateLabel}</span>
      </header>

      {snapshot.windows.length > 0 ? <div className="windows">
        {snapshot.windows.map(window => {
          const status = windowState(snapshot, window, now);
          const value = showUsed ? window.usedPercent : remaining(window.usedPercent);
          const usable = status !== 'awaiting' && status !== 'unavailable';
          const low = (remaining(window.usedPercent) ?? 100) <= 20;
          return <div className={`usage-window ${status} ${low ? 'low' : ''}`} key={window.id}>
            <div className="window-heading">
              <span>{window.label}</span>
              <strong>{usable && value !== null ? `${Math.round(value)}%` : status === 'awaiting' ? 'Updating' : '—'}</strong>
            </div>
            <div className="meter" role="progressbar" aria-label={`${window.label} ${showUsed ? 'used' : 'remaining'}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={usable && value !== null ? Math.round(value) : undefined}>
              <div style={{ width: usable ? `${value ?? 0}%` : '0%' }} />
            </div>
            <div className="window-meta"><span><Clock3 size={10} />{countdown(window.resetsAt, now, window.usedPercent)}</span><span>{status === 'stale' ? 'Last known' : showUsed ? 'used' : 'remaining'}</span></div>
          </div>;
        })}
      </div> : <div className="empty-provider">
        <p>{snapshot.message ?? 'No allowance data is available.'}</p>
        {snapshot.state === 'disconnected' && <button className="connect-button" disabled={busy} onClick={() => onConnect(snapshot.provider, true)}><Plug size={13} />Connect</button>}
      </div>}

      <footer className="provider-footer" title={snapshot.observedAt ? new Date(snapshot.observedAt * 1000).toLocaleString() : snapshot.source}>
        <span className={`tiny-dot ${fresh === 'fresh' ? 'live' : ''}`} />{snapshot.observedAt ? age(snapshot.observedAt, now) : stateLabel}
      </footer>
    </article>
  );
}

export default function App() {
  const [view, setView] = useState<View | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [now, setNow] = useState(Date.now() / 1000);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState('');
  const [claudeConfirm, setClaudeConfirm] = useState(false);

  const load = useCallback(async () => {
    setView(await call<View>('get_state'));
    setNow(Date.now() / 1000);
  }, []);

  useEffect(() => {
    load().catch(() => setNotice('Could not connect to the desktop service.'));
    const timer = setInterval(() => setNow(Date.now() / 1000), 15_000);
    let stop: (() => void) | undefined;
    let cancelled = false;
    if (desktop) listen('usage-updated', () => {
      setNotice(current => current === 'Refreshing usage…' ? '' : current);
      void load();
    }).then(unlisten => cancelled ? unlisten() : stop = unlisten);
    return () => { cancelled = true; clearInterval(timer); stop?.(); };
  }, [load]);

  useEffect(() => {
    if (notice !== 'Refreshing usage…') return;
    const timer = window.setTimeout(() => setNotice(current => current === 'Refreshing usage…' ? '' : current), 4_000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  useEffect(() => { document.documentElement.dataset.theme = view?.settings.theme ?? 'system'; }, [view?.settings.theme]);
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      if (settingsOpen) setSettingsOpen(false);
      else if (desktop) void getCurrentWindow().hide();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [settingsOpen]);

  async function action(fn: () => Promise<unknown>) {
    setBusy(true);
    try { await fn(); await load(); }
    catch (error) { setNotice(String(error)); }
    finally { setBusy(false); }
  }

  async function prefs(patch: Partial<Settings>) {
    if (!view) return;
    await action(async () => {
      const next = { ...view.settings, ...patch };
      if (next.alerts && !view.settings.alerts && desktop) {
        let granted = await isPermissionGranted();
        if (!granted) granted = await requestPermission() === 'granted';
        if (!granted) throw new Error('Allow notifications in system settings before enabling alerts.');
      }
      await call('update_preferences', { theme: next.theme, showUsed: next.showUsed, alerts: next.alerts });
    });
  }

  async function connect(id: ProviderId, enabled: boolean) {
    if (id === 'claude' && enabled && !claudeConfirm) {
      setClaudeConfirm(true);
      setNotice('Connecting Claude Code updates your user-level statusLine after backing it up. Choose Connect again to approve.');
      return;
    }
    setClaudeConfirm(false);
    await action(async () => setNotice(await call<string>('connect_provider', { provider: id, enabled })));
  }

  const ready = view?.providers.filter(snapshot => snapshot.state === 'ready' && freshness(snapshot, now) === 'fresh').length ?? 0;

  return <main className="app-shell">
    <header className="tray-header">
      <div className="brand"><Gauge size={17} /><span>Allowance</span><span className={`tiny-dot ${ready ? 'live' : ''}`} /><small>{ready}/2 live</small></div>
      <div className="header-actions">
        <button className={`icon-button ${busy ? 'spinning' : ''}`} disabled={busy} aria-label="Refresh usage" title="Refresh usage" onClick={() => void action(async () => { setNotice('Refreshing usage…'); await call('refresh'); })}><RefreshCw size={16} /></button>
        <button className={`icon-button ${settingsOpen ? 'selected' : ''}`} aria-label="Preferences" onClick={() => setSettingsOpen(!settingsOpen)}><Settings2 size={15} /></button>
        <button className="icon-button" aria-label="Close usage tray" onClick={() => desktop ? void getCurrentWindow().hide() : undefined}><X size={16} /></button>
      </div>
    </header>

    {!desktop && <div className="preview-banner">TRAY PREVIEW <span>Sample data</span></div>}

    <div className="tray-body" inert={settingsOpen}>
      <div className="display-row">
        <span>Usage</span>
        <div className="segmented" aria-label="Usage display">
          <button aria-pressed={!view?.settings.showUsed} disabled={busy} onClick={() => void prefs({ showUsed: false })}>Remaining</button>
          <button aria-pressed={view?.settings.showUsed ?? false} disabled={busy} onClick={() => void prefs({ showUsed: true })}>Used</button>
        </div>
      </div>
      {notice && <div className="notice" role="status"><span>{notice}</span><button aria-label="Dismiss message" onClick={() => setNotice('')}><X size={13} /></button></div>}
      {view ? <div className="provider-list">{view.providers.map(snapshot => <ProviderCard key={snapshot.provider} snapshot={snapshot} showUsed={view.settings.showUsed} now={now} busy={busy} onConnect={(id, enabled) => void connect(id, enabled)} />)}</div> : <div className="loading"><Gauge size={24} /><span>Finding your accounts…</span></div>}
    </div>

    {settingsOpen && view && <div className="settings-layer">
      <section className="settings-panel" role="dialog" aria-modal="true" aria-labelledby="settings-title">
        <header><h2 id="settings-title">Preferences</h2><button className="icon-button" aria-label="Close preferences" onClick={() => setSettingsOpen(false)} autoFocus><X size={16} /></button></header>
        <div className="settings-body">
          <div className="setting-section"><label>Appearance</label><div className="theme-options">{(['system', 'light', 'dark'] as const).map((theme, index) => { const Icon = [Laptop, Sun, Moon][index]; return <button key={theme} aria-pressed={view.settings.theme === theme} disabled={busy} onClick={() => void prefs({ theme })}><Icon size={15} />{theme[0].toUpperCase() + theme.slice(1)}{view.settings.theme === theme && <Check size={11} />}</button>; })}</div></div>
          <label className="setting-row"><span><strong><Bell size={14} />Low allowance alerts</strong><small>Notify at 20% and 10% remaining.</small></span><input type="checkbox" checked={view.settings.alerts} disabled={busy} onChange={event => void prefs({ alerts: event.target.checked })} /></label>
          <label className="setting-row"><span><strong><Power size={14} />Launch at login</strong><small>Start quietly in the menu bar or system tray.</small></span><input type="checkbox" checked={view.autostart} disabled={busy} onChange={event => void action(() => call('set_autostart', { enabled: event.target.checked }))} /></label>
          <div className="setting-section"><label>Connected accounts</label>{(['claude', 'codex'] as const).map(id => { const enabled = id === 'claude' ? view.settings.claudeEnabled : view.settings.codexEnabled; return <div className="connection-row" key={id}><ProviderMark id={id} /><span>{names[id]}</span><button disabled={busy} onClick={() => void connect(id, !enabled)}>{enabled ? 'Disconnect' : id === 'claude' && claudeConfirm ? 'Approve & connect' : 'Connect'}</button></div>; })}<p className="settings-help">Claude Code connection backs up and extends your user-level statusLine. It refreshes from CLI and local Desktop sessions; restart sessions opened before connecting. Remote sessions cannot report usage to this device.</p></div>
          <button className="quit-button" onClick={() => void call('quit')}><Power size={14} />Quit Allowance</button>
        </div>
      </section>
    </div>}
  </main>;
}
