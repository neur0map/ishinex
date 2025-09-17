export type ProviderId = 'claude' | 'codex' | 'gemini';

export type ProviderConfig = {
  id: ProviderId;
  name: string;
  eventPrefix: string; // e.g., 'claude', 'codex', 'gemini'
  defaultModel: string;
  models: { id: string; name: string }[]; // basic static list; could be dynamic later
};

export const PROVIDERS: ProviderConfig[] = [
  {
    id: 'claude',
    name: 'Claude Code',
    eventPrefix: 'claude',
    defaultModel: 'sonnet',
    models: [
      { id: 'sonnet', name: 'Claude 4 Sonnet' },
      { id: 'opus', name: 'Claude 4 Opus' },
    ],
  },
  {
    id: 'codex',
    name: 'OpenAI Codex',
    eventPrefix: 'codex',
    defaultModel: 'o4-mini',
    models: [
      { id: 'o4-mini', name: 'o4-mini' },
      { id: 'gpt-4o', name: 'GPT-4o' },
      { id: 'gpt-4o-mini', name: 'GPT-4o mini' },
    ],
  },
  {
    id: 'gemini',
    name: 'Google Gemini',
    eventPrefix: 'gemini',
    defaultModel: 'gemini-1.5-pro',
    models: [
      { id: 'gemini-1.5-pro', name: 'Gemini 1.5 Pro' },
      { id: 'gemini-1.5-flash', name: 'Gemini 1.5 Flash' },
    ],
  },
];

export function getProvider(id: ProviderId): ProviderConfig {
  const p = PROVIDERS.find(p => p.id === id);
  if (!p) throw new Error(`Unknown provider: ${id}`);
  return p;
}
