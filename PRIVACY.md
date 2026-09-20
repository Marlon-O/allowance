# Privacy

Allowance processes provider allowance data locally. It has no application-operated server, analytics, advertising, crash reporting, or cross-device sync.

## Data accessed

- Codex account identity and rate-limit windows exposed by the locally authenticated Codex app server
- Claude account identity and usage windows emitted through Claude Code's documented status-line input
- Claude usage percentages and reset timestamps from Claude Desktop's local plan-usage history and HTTP cache
- Local app preferences, notification state, and window position

## Data stored

- Settings and connection state
- The visible account label when a provider supplies one
- The latest Claude Code allowance snapshot needed to display current usage
- A backup of the prior Claude `statusLine` setting while Claude integration is connected

Allowance does not store provider credentials, API keys, prompts, transcript contents, conversation content, the complete Claude status-line payload, or Claude's cached HTTP response. It extracts only the usage percentages and reset timestamps it displays. Authentication remains inside the official provider tools.

Users can disconnect providers in Preferences. Disconnect Claude before deleting app data so its status-line backup can be restored. Uninstalling the app does not sign out of Claude Code or Codex.

Operating systems and provider CLIs have their own privacy terms. Contributors should disclose any future network service or telemetry before enabling it, provide explicit consent, and update this document.
