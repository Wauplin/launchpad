// Settings part of the demo world: harness descriptors and models, devices, terminals, sign-in checks, usage history
// and the SSH configuration. Status, configuration and profiles live in `core.ts`.

import type { AgentRun } from "@/gen/AgentRun";
import type { Check } from "@/gen/Check";
import type { HarnessDefaults } from "@/gen/HarnessDefaults";
import type { HarnessDescriptor } from "@/gen/HarnessDescriptor";
import type { HostStatus } from "@/gen/HostStatus";
import type { ModelInfo } from "@/gen/ModelInfo";
import type { Profile } from "@/gen/Profile";
import type { SignInCheck } from "@/gen/SignInCheck";
import type { SshConfigHost } from "@/gen/SshConfigHost";
import type { TerminalInfo } from "@/gen/TerminalInfo";
import type { WeeklyReset } from "@/gen/WeeklyReset";
import { clock } from "../daemon";
import { P, ROOT, USER } from "./ids";

/** A paired browser, as `GET /api/devices` returns it (Unix seconds). */
export type DemoDevice = { id: string; name: string; created_at: number; expires_at: number; last_seen_at: number | null; push: boolean; current: boolean };
/** `GET /api/setup/trace-backup`. */
export type DemoTraceBackup = { check: SignInCheck; account: string | null; error: string | null; last_upload: number | null };

export const HOME = "/home/demo";

/**
 * `GET /api/harnesses`, copied from the adapters' descriptors (`crates/core/src/harness/{claude,codex,opencode}.rs`,
 * icons from `harness/icons/`): capabilities, environment variables and profile templates are the real ones.
 */
const HARNESSES: HarnessDescriptor[] = [
  {"id": "claude", "name": "Claude Code", "icon_svg": "<svg height=\"1em\" style=\"flex:none;line-height:1\" viewBox=\"0 0 24 24\" width=\"1em\" xmlns=\"http://www.w3.org/2000/svg\"><title>Claude</title><path d=\"M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z\" fill=\"#D97757\" fill-rule=\"nonzero\"></path></svg>", "color": "#b46a4b", "working_message": "Message Claude. It is read after the current step.", "capabilities": {"permission_modes": true, "images": true, "messages_while_running": true, "terminal": true, "quota": true, "cost": true}, "browser_isolation": "Browser tools only; built-in tools and other MCP servers disabled", "cost_model_prefixes": ["claude-", "opus", "sonnet", "haiku"], "account_env": ["CLAUDE_CONFIG_DIR"], "env_vars": [{"name": "CLAUDE_CONFIG_DIR", "group": "Account", "description": "Configuration folder with its own sign-in, settings and history. Use one folder per account; sign in once with: CLAUDE_CONFIG_DIR=<folder> claude", "default": "~/.claude", "example": null, "secret": false}, {"name": "ANTHROPIC_API_KEY", "group": "Account", "description": "Anthropic Console API key, sent as X-Api-Key. Replaces the subscription sign-in and is billed per use.", "default": null, "example": "sk-ant-…", "secret": true}, {"name": "ANTHROPIC_AUTH_TOKEN", "group": "Account", "description": "Bearer token for a gateway or proxy (Authorization header). Takes precedence over ANTHROPIC_API_KEY.", "default": null, "example": "${HF_TOKEN}", "secret": true}, {"name": "CLAUDE_CODE_OAUTH_TOKEN", "group": "Account", "description": "Subscription token created with: claude setup-token", "default": null, "example": "sk-ant-oat01-…", "secret": true}, {"name": "ANTHROPIC_BASE_URL", "group": "Endpoint", "description": "API endpoint of a gateway or compatible provider. Claude Code appends /v1/messages.", "default": "https://api.anthropic.com", "example": null, "secret": false}, {"name": "ANTHROPIC_CUSTOM_HEADERS", "group": "Endpoint", "description": "Extra request headers, one Name: Value per line.", "default": null, "example": "X-HF-Bill-To: my-org", "secret": false}, {"name": "HTTPS_PROXY", "group": "Endpoint", "description": "HTTPS proxy for API requests.", "default": null, "example": "http://proxy.example.com:8080", "secret": false}, {"name": "NO_PROXY", "group": "Endpoint", "description": "Hosts that bypass the proxy.", "default": null, "example": "localhost,127.0.0.1", "secret": false}, {"name": "CLAUDE_CODE_USE_BEDROCK", "group": "Cloud provider", "description": "Use Amazon Bedrock, with the AWS credentials of the environment.", "default": null, "example": "1", "secret": false}, {"name": "AWS_REGION", "group": "Cloud provider", "description": "Bedrock region, when the AWS profile has none.", "default": null, "example": "us-east-1", "secret": false}, {"name": "AWS_PROFILE", "group": "Cloud provider", "description": "AWS profile used for Bedrock.", "default": null, "example": "default", "secret": false}, {"name": "AWS_BEARER_TOKEN_BEDROCK", "group": "Cloud provider", "description": "Bedrock API key.", "default": null, "example": null, "secret": true}, {"name": "CLAUDE_CODE_USE_VERTEX", "group": "Cloud provider", "description": "Use Google Vertex AI, with the gcloud credentials of the environment.", "default": null, "example": "1", "secret": false}, {"name": "ANTHROPIC_VERTEX_PROJECT_ID", "group": "Cloud provider", "description": "Google Cloud project for Vertex AI.", "default": null, "example": "my-project", "secret": false}, {"name": "CLOUD_ML_REGION", "group": "Cloud provider", "description": "Vertex AI region.", "default": null, "example": "global", "secret": false}, {"name": "CLAUDE_CODE_USE_FOUNDRY", "group": "Cloud provider", "description": "Use Microsoft Foundry.", "default": null, "example": "1", "secret": false}, {"name": "ANTHROPIC_FOUNDRY_RESOURCE", "group": "Cloud provider", "description": "Foundry resource name.", "default": null, "example": "my-resource", "secret": false}, {"name": "ANTHROPIC_FOUNDRY_API_KEY", "group": "Cloud provider", "description": "Foundry API key.", "default": null, "example": null, "secret": true}, {"name": "ANTHROPIC_MODEL", "group": "Models", "description": "Model of the session. The model chosen in Launchpad (--model) takes precedence.", "default": null, "example": "claude-sonnet-5-5", "secret": false}, {"name": "ANTHROPIC_DEFAULT_OPUS_MODEL", "group": "Models", "description": "Model used for the opus alias. ${LAUNCHPAD_MODEL} is the model chosen in Launchpad.", "default": "latest Opus", "example": "${LAUNCHPAD_MODEL}", "secret": false}, {"name": "ANTHROPIC_DEFAULT_SONNET_MODEL", "group": "Models", "description": "Model used for the sonnet alias.", "default": "latest Sonnet", "example": "${LAUNCHPAD_MODEL}", "secret": false}, {"name": "ANTHROPIC_DEFAULT_HAIKU_MODEL", "group": "Models", "description": "Model used for the haiku alias and for background tasks.", "default": "latest Haiku", "example": "${LAUNCHPAD_MODEL}", "secret": false}, {"name": "ANTHROPIC_DEFAULT_FABLE_MODEL", "group": "Models", "description": "Model used for the fable alias.", "default": "latest Fable", "example": "${LAUNCHPAD_MODEL}", "secret": false}, {"name": "CLAUDE_CODE_SUBAGENT_MODEL", "group": "Models", "description": "Default model of subagents.", "default": "inherits the session model", "example": "${LAUNCHPAD_MODEL}", "secret": false}, {"name": "CLAUDE_CODE_MAX_CONTEXT_TOKENS", "group": "Models", "description": "Context window assumed for model IDs Claude Code does not know.", "default": "200000", "example": null, "secret": false}, {"name": "CLAUDE_CODE_MAX_OUTPUT_TOKENS", "group": "Limits", "description": "Maximum output tokens per response.", "default": "32000 (model dependent)", "example": null, "secret": false}, {"name": "MAX_THINKING_TOKENS", "group": "Limits", "description": "Fixed thinking budget; 0 disables thinking on most models.", "default": null, "example": "8000", "secret": false}, {"name": "API_TIMEOUT_MS", "group": "Limits", "description": "Timeout of one API request, in milliseconds.", "default": "600000", "example": null, "secret": false}, {"name": "CLAUDE_CODE_MAX_RETRIES", "group": "Limits", "description": "Retries of a failed API request.", "default": "10", "example": null, "secret": false}, {"name": "CLAUDE_CODE_DISABLE_THINKING", "group": "Compatibility", "description": "Omit the thinking parameter, for gateways that reject it.", "default": null, "example": "1", "secret": false}, {"name": "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "group": "Compatibility", "description": "Remove beta headers and fields, for gateways that reject them.", "default": null, "example": "1", "secret": false}, {"name": "DISABLE_PROMPT_CACHING", "group": "Compatibility", "description": "Turn off prompt caching.", "default": null, "example": "1", "secret": false}, {"name": "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "group": "Privacy", "description": "Any value turns off auto-updates, telemetry, error reporting and feedback.", "default": null, "example": "1", "secret": false}, {"name": "DISABLE_TELEMETRY", "group": "Privacy", "description": "Any value, even 0, turns off telemetry.", "default": null, "example": "1", "secret": false}, {"name": "DISABLE_ERROR_REPORTING", "group": "Privacy", "description": "Any value, even 0, turns off error reporting.", "default": null, "example": "1", "secret": false}], "profile_templates": [{"kind": "subscription", "name": "Subscription", "description": "Your Claude Pro or Max login (claude auth login), protected by its 5-hour and weekly quota.", "setup": null, "setup_command": null, "env": [], "billing": "subscription", "models_url": null}, {"kind": "subscription", "name": "Another account", "description": "A second subscription or organization, with its own sign-in in a separate configuration folder and its own quota meters.", "setup": "Save the profile, then sign in once from Works on.", "setup_command": null, "env": [["CLAUDE_CONFIG_DIR", "~/.claude-work"]], "billing": "subscription", "models_url": null}, {"kind": "provider", "name": "Hugging Face", "description": "Open models (Kimi, GLM, Qwen, DeepSeek…) through Hugging Face Inference Providers, like the hf claude extension. Billed to your Hugging Face account.", "setup": "Launchpad reads HF_TOKEN, else the token saved by the hf CLI. Sign in once if needed:", "setup_command": "hf auth login", "env": [["ANTHROPIC_BASE_URL", "https://router.huggingface.co"], ["ANTHROPIC_AUTH_TOKEN", "${HF_TOKEN}"], ["ANTHROPIC_DEFAULT_OPUS_MODEL", "${LAUNCHPAD_MODEL}"], ["ANTHROPIC_DEFAULT_SONNET_MODEL", "${LAUNCHPAD_MODEL}"], ["ANTHROPIC_DEFAULT_HAIKU_MODEL", "${LAUNCHPAD_MODEL}"], ["ANTHROPIC_DEFAULT_FABLE_MODEL", "${LAUNCHPAD_MODEL}"], ["CLAUDE_CODE_SUBAGENT_MODEL", "${LAUNCHPAD_MODEL}"]], "billing": "pay_as_you_go", "models_url": "https://router.huggingface.co/v1/models"}, {"kind": "api_key", "name": "API key", "description": "Pay per use with a Console API key instead of the subscription.", "setup": "Create a key at https://console.anthropic.com/settings/keys and paste it below.", "setup_command": null, "env": [["ANTHROPIC_API_KEY", ""]], "billing": "pay_as_you_go", "models_url": null}]},
  {"id": "codex", "name": "Codex", "icon_svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><title>Codex</title><defs><linearGradient id=\"codex-g\" gradientUnits=\"userSpaceOnUse\" x1=\"12\" x2=\"12\" y1=\"0\" y2=\"24\"><stop stop-color=\"#B1A7FF\"/><stop offset=\".5\" stop-color=\"#7A9DFF\"/><stop offset=\"1\" stop-color=\"#3941FF\"/></linearGradient></defs><path fill=\"url(#codex-g)\" fill-rule=\"evenodd\" clip-rule=\"evenodd\" d=\"M8.086.457a6.105 6.105 0 013.046-.415c1.333.153 2.521.72 3.564 1.7a.117.117 0 00.107.029c1.408-.346 2.762-.224 4.061.366l.063.03.154.076c1.357.703 2.33 1.77 2.918 3.198.278.679.418 1.388.421 2.126a5.655 5.655 0 01-.18 1.631.167.167 0 00.04.155 5.982 5.982 0 011.578 2.891c.385 1.901-.01 3.615-1.183 5.14l-.182.22a6.063 6.063 0 01-2.934 1.851.162.162 0 00-.108.102c-.255.736-.511 1.364-.987 1.992-1.199 1.582-2.962 2.462-4.948 2.451-1.583-.008-2.986-.587-4.21-1.736a.145.145 0 00-.14-.032c-.518.167-1.04.191-1.604.185a5.924 5.924 0 01-2.595-.622 6.058 6.058 0 01-2.146-1.781c-.203-.269-.404-.522-.551-.821a7.74 7.74 0 01-.495-1.283 6.11 6.11 0 01-.017-3.064.166.166 0 00.008-.074.115.115 0 00-.037-.064 5.958 5.958 0 01-1.38-2.202 5.196 5.196 0 01-.333-1.589 6.915 6.915 0 01.188-2.132c.45-1.484 1.309-2.648 2.577-3.493.282-.188.55-.334.802-.438.286-.12.573-.22.861-.304a.129.129 0 00.087-.087A6.016 6.016 0 015.635 2.31C6.315 1.464 7.132.846 8.086.457zm-.804 7.85a.848.848 0 00-1.473.842l1.694 2.965-1.688 2.848a.849.849 0 001.46.864l1.94-3.272a.849.849 0 00.007-.854l-1.94-3.393zm5.446 6.24a.849.849 0 000 1.695h4.848a.849.849 0 000-1.696h-4.848z\"/></svg>\n", "color": "#3b82f6", "working_message": "Message Codex. It steers the running turn.", "capabilities": {"permission_modes": true, "images": true, "messages_while_running": true, "terminal": true, "quota": true, "cost": false}, "browser_isolation": "Read-only sandbox; native shell tools may remain available", "cost_model_prefixes": [], "account_env": ["CODEX_HOME"], "env_vars": [{"name": "CODEX_HOME", "group": "Account", "description": "Folder with config.toml, sign-in (auth.json), history and logs. Use one folder per account or provider; sign in once with: CODEX_HOME=<folder> codex login", "default": "~/.codex", "example": null, "secret": false}, {"name": "CODEX_SQLITE_HOME", "group": "Account", "description": "Folder of the SQLite state database.", "default": "$CODEX_HOME", "example": null, "secret": false}, {"name": "OPENAI_API_KEY", "group": "Account", "description": "Read only by providers configured with env_key = \"OPENAI_API_KEY\" in config.toml; for the built-in provider, run: codex login --with-api-key", "default": null, "example": "sk-…", "secret": true}, {"name": "OPENAI_ORGANIZATION", "group": "Account", "description": "OpenAI organization sent with API requests (OpenAI-Organization header).", "default": null, "example": "org-…", "secret": false}, {"name": "OPENAI_PROJECT", "group": "Account", "description": "OpenAI project sent with API requests (OpenAI-Project header).", "default": null, "example": "proj_…", "secret": false}, {"name": "CODEX_OSS_BASE_URL", "group": "Endpoint", "description": "Base URL of the local ollama or lmstudio provider, when config.toml selects one.", "default": "http://localhost:11434/v1", "example": null, "secret": false}, {"name": "CODEX_CA_CERTIFICATE", "group": "Endpoint", "description": "PEM file with extra CA certificates, for a corporate proxy.", "default": "$SSL_CERT_FILE", "example": "/etc/ssl/certs/corp.pem", "secret": false}, {"name": "HTTPS_PROXY", "group": "Endpoint", "description": "HTTPS proxy for API requests.", "default": null, "example": "http://proxy.example.com:8080", "secret": false}, {"name": "NO_PROXY", "group": "Endpoint", "description": "Hosts that bypass the proxy.", "default": null, "example": "localhost,127.0.0.1", "secret": false}, {"name": "RUST_LOG", "group": "Debugging", "description": "Log filter of the Codex process, written under CODEX_HOME/log.", "default": null, "example": "codex_core=debug", "secret": false}], "profile_templates": [{"kind": "subscription", "name": "ChatGPT subscription", "description": "Your ChatGPT Plus or Pro login (codex login), protected by its 5-hour and weekly quota.", "setup": null, "setup_command": null, "env": [], "billing": "subscription", "models_url": null}, {"kind": "subscription", "name": "Another ChatGPT account", "description": "A second ChatGPT account, with its own sign-in in a separate CODEX_HOME and its own quota meters.", "setup": "Save the profile, then sign in once from Works on.", "setup_command": null, "env": [["CODEX_HOME", "~/.codex-work"]], "billing": "subscription", "models_url": null}, {"kind": "api_key", "name": "OpenAI API key", "description": "Pay per use with an OpenAI API key, stored by Codex in a separate CODEX_HOME. Put a custom model provider in its config.toml to use another provider.", "setup": "Run this once in a terminal, paste the key, then press Enter and Ctrl+D:", "setup_command": "mkdir -p ~/.codex-api && CODEX_HOME=~/.codex-api codex login --with-api-key", "env": [["CODEX_HOME", "~/.codex-api"]], "billing": "pay_as_you_go", "models_url": null}]},
  {"id": "opencode", "name": "OpenCode", "icon_svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><path fill=\"#4b5563\" d=\"M4 3h16v18H4zm4 4v10h8V7zm2 2h4v6h-4z\" fill-rule=\"evenodd\"/></svg>\n", "color": "#4b5563", "working_message": "OpenCode is working. Wait for this turn to finish before sending another message.", "capabilities": {"permission_modes": false, "images": true, "messages_while_running": false, "terminal": true, "quota": false, "cost": true}, "browser_isolation": "Native tools restricted by OpenCode permissions; not an OS sandbox", "cost_model_prefixes": [], "account_env": [], "env_vars": [], "profile_templates": [{"kind": "provider", "name": "OpenCode", "description": "The providers and models configured in OpenCode itself (opencode auth login), billed by each provider.", "setup": null, "setup_command": null, "env": [], "billing": "pay_as_you_go", "models_url": null}]},
];

const efforts = ["low", "medium", "high", "max"];
const model = (id: string, display_name: string, description: string, modelEfforts: string[] = efforts): ModelInfo => ({ id, display_name, description, efforts: modelEfforts, providers: [] });

/** `GET /api/harness/{kind}/defaults`: neutral model names that do not go stale. */
const DEFAULTS: Record<string, HarnessDefaults> = {
  claude: {
    model: null, effort: null, efforts,
    models: [
      model("model-large", "Large", "Most capable, for complex and long tasks"),
      model("model-medium", "Medium", "Balanced, for everyday coding"),
      model("model-fast", "Fast", "Fastest, for quick answers and small edits"),
    ],
  },
  codex: {
    model: null, effort: null, efforts: ["low", "medium", "high"],
    models: [
      model("coder-large", "Coder Large", "Frontier coding model for demanding work", ["low", "medium", "high"]),
      model("coder-medium", "Coder Medium", "Workhorse model for everyday coding", ["low", "medium", "high"]),
      model("coder-fast", "Coder Fast", "Fast and affordable model for easier tasks", ["low", "medium"]),
    ],
  },
  opencode: {
    model: null, effort: null, efforts: [],
    models: [
      model("opencode/open-coder-free", "opencode/open-coder-free", "", []),
      model("router/open-coder-70b", "router/open-coder-70b", "", []),
      model("router/open-reasoner-32b", "router/open-reasoner-32b", "", []),
    ],
  },
};

/** `POST /api/profiles/models`: the OpenAI-compatible model list of the pay-as-you-go router profile, with providers. */
const ROUTER_MODELS: ModelInfo[] = [
  {
    id: "open-labs/open-coder-70b", display_name: "open-labs/open-coder-70b", description: "Open coding model, tool calls", efforts: [],
    providers: [
      { name: "provider-one", context_length: 262144, input_usd: 0.6, cached_input_usd: 0.15, output_usd: 2.4, throughput: 88, first_token_ms: 420 },
      { name: "provider-two", context_length: 131072, input_usd: 0.45, cached_input_usd: null, output_usd: 1.9, throughput: 41, first_token_ms: 910 },
    ],
  },
  {
    id: "open-labs/open-reasoner-32b", display_name: "open-labs/open-reasoner-32b", description: "Reasoning model, tool calls", efforts: [],
    providers: [
      { name: "provider-one", context_length: 131072, input_usd: 0.3, cached_input_usd: 0.08, output_usd: 1.2, throughput: 120, first_token_ms: 300 },
      { name: "provider-three", context_length: 65536, input_usd: 0.2, cached_input_usd: null, output_usd: 0.8, throughput: 64, first_token_ms: 650 },
    ],
  },
  {
    id: "small-ai/tiny-coder-8b", display_name: "small-ai/tiny-coder-8b", description: "Small and fast, tool calls", efforts: [],
    providers: [{ name: "provider-two", context_length: 32768, input_usd: 0.05, cached_input_usd: null, output_usd: 0.1, throughput: 210, first_token_ms: 180 }],
  },
];

/** A small deterministic generator, so every run of the demo shows the same history. */
export function seeded(seed: number) {
  let s = seed >>> 0;
  return () => { s = (s * 1664525 + 1013904223) >>> 0; return s / 2 ** 32; };
}

/** Agent starts of the last 60 days, for Settings → Usage (`GET /api/analytics/runs`). */
function agentRuns(): AgentRun[] {
  const rand = seeded(7);
  const runs: AgentRun[] = [];
  const series: [string, string | null, number][] = [["claude", "claude-max", 0.55], ["codex", "codex-plus", 0.3], ["opencode", "opencode-router", 0.15]];
  for (let day = 60; day >= 0; day--) {
    const weekend = [0, 6].includes(new Date(clock.ago(day * 24 * 60)).getDay());
    const count = Math.floor(rand() * (weekend ? 2 : 7));
    for (let i = 0; i < count; i++) {
      let pick = rand();
      const [harness, profile] = series.find(([, , w]) => (pick -= w) < 0) ?? series[0]!;
      const input = Math.round(20_000 + rand() * 400_000);
      runs.push({ started_at: clock.ago(day * 24 * 60 + 9 * 60 - Math.round(rand() * 600)), harness, profile, input_tokens: input, cached_tokens: Math.round(input * (2 + rand() * 6)), output_tokens: Math.round(input * (0.05 + rand() * 0.1)) });
    }
  }
  return runs.sort((a, b) => a.started_at - b.started_at);
}

/** Weekly quota resets of the two subscription logins over the last eight weeks (`GET /api/quota-history`). */
function quotaHistory(): WeeklyReset[] {
  const rand = seeded(11);
  const out: WeeklyReset[] = [];
  for (const [harness, offsetDays] of [["claude", 2.3], ["codex", 4.6]] as const) {
    for (let week = 0; week < 8; week++) {
      const observed = clock.ago((offsetDays + week * 7) * 24 * 60);
      out.push({ harness, observed_at: observed, previous_at: observed - Math.round((10 + rand() * 80) * 60_000), previous_percent: Math.round(45 + rand() * 50), used_percent: Math.round(rand() * 3) });
    }
  }
  return out.sort((a, b) => a.observed_at - b.observed_at);
}

const seconds = (ms: number) => Math.floor(ms / 1000);

export function settingsWorld() {
  const harnesses: HarnessDescriptor[] = structuredClone(HARNESSES);
  const harnessDefaults: Record<string, HarnessDefaults> = structuredClone(DEFAULTS);
  const routerModels: ModelInfo[] = structuredClone(ROUTER_MODELS);
  const devices: DemoDevice[] = [
    { id: "demo-device-desktop", name: "This desktop", created_at: seconds(clock.ago(60 * 24 * 40)), expires_at: seconds(clock.ago(-60 * 24 * 325)), last_seen_at: seconds(clock.now()), push: false, current: true },
    { id: "demo-device-phone", name: "Phone", created_at: seconds(clock.ago(60 * 24 * 12)), expires_at: seconds(clock.ago(-60 * 24 * 353)), last_seen_at: seconds(clock.ago(95)), push: true, current: false },
  ];
  // One recorded shell exists from the start ("Terminals · 1" in the status bar); its replay starts when it is shown.
  const terminals: TerminalInfo[] = [
    { id: "demo-term-billing", kind: "shell", title: "billing-api · main", cwd: `${ROOT}/billing-api`, project_id: P.billing, session_id: null, alive: true, exit_code: null, created_at: clock.ago(12), folder: "billing-api", place: "checkout", branch: "main", setup: false, busy: false, notice: null },
  ];
  const gh: SignInCheck = { id: "github", profile: null, name: "GitHub CLI", cli_found: true, signed_in: true, detail: `Logged in to github.com account ${USER}`, command: "gh auth login" };
  const traceBackup: DemoTraceBackup = {
    check: { id: "huggingface", profile: null, name: "Hugging Face CLI", cli_found: true, signed_in: true, detail: `Account ${USER}`, command: "hf auth login" },
    account: USER, error: null, last_upload: null,
  };
  const sshConfig: SshConfigHost[] = [
    { alias: "gpu-box", hostname: "gpu-box.lan", user: "demo", proxy_jump: null },
    { alias: "build-runner", hostname: "10.0.4.17", user: "ci", proxy_jump: "bastion" },
  ];
  return { harnesses, harnessDefaults, routerModels, devices, terminals, githubCheck: gh, traceBackup, sshConfig, agentRuns: agentRuns(), quotaHistory: quotaHistory(), homeFolder: HOME };
}

/** Preflight checks of a connected demo host (`POST /api/hosts/{id}/checks`), with one entry per profile that has an environment. */
export function hostChecks(destination: string, profiles: Profile[]): Check[] {
  const check = (id: string, name: string, detail: string, extra: Partial<Check> = {}): Check => ({ id, name, ok: true, detail, command: null, found: null, signed_in: null, ...extra });
  return [
    check("ssh", "SSH connection", `to ${destination} · 18 ms · system OpenSSH with your ~/.ssh/config`),
    check("system", "System", "Linux x86_64 · user demo"),
    check("helper", "Launchpad helper", "Installed in ~/.local/share/launchpad-host, build demo (same as this daemon)"),
    check("git", "Git", "git version 2.x"),
    check("gh", "GitHub CLI", `Signed in as ${USER} · publication and merges push from this host`),
    check("claude", "Claude Code", "2.1.0 · signed in · same account as this computer", { found: true, signed_in: true }),
    check("codex", "Codex", "0.80.0 · signed in · same account as this computer", { found: true, signed_in: true }),
    check("opencode", "OpenCode", "1.2.0 · signed in", { found: true, signed_in: true }),
    ...profiles.filter((p) => Object.keys(p.env).length).map((p) => check(`profile:${p.id}`, p.name, "signed in", { found: true, signed_in: true })),
  ];
}

/** The `gpu-box` SSH machine, connected and checked. */
export function demoHost(id: string, destination: string, profiles: Profile[], projectsRoot = "~/projects"): HostStatus {
  return {
    host: { id, destination, port: null, projects_root: projectsRoot, created_at: clock.ago(60 * 24 * 20) },
    state: "connected", error: null, since: clock.ago(42),
    info: { os: "Linux", arch: "x86_64", home: HOME, user: "demo", shell: "/bin/bash", cli: `${HOME}/.local/share/launchpad-host/bin/demo/launchpad` },
    latency_ms: 18, prompt: null, checks: hostChecks(destination, profiles), checked_at: clock.ago(41),
  };
}
