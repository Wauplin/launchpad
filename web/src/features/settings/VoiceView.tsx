// Settings → Voice: dictation in the message boxes. Shared choices go to config.toml (`[voice]`, PATCH /api/config):
// on or off, the sources in fallback order (Claude subscription profiles; their logins stay in Home's vault), the
// language and when to stop after silence. This device's microphone and sounds stay in the browser.
// Test voice records 3 s with each source in order and shows what it heard and how soon the first words came.

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import type { VoiceConfig } from "@/gen/VoiceConfig";
import type { VoiceSource } from "@/gen/VoiceSource";
import { Icon } from "@/components/Icon";
import { VoiceCaption } from "@/components/VoiceCaption";
import { MenuItem } from "@/components/Popover";
import { Badge, Choice, Choices, IconButton, Loading, MoreMenu, SettingRow, useSaveState, type BadgeTone } from "@/components/ui";
import { isDemo } from "@/demo/mode";
import { usePhone } from "@/lib/phone";
import { when } from "@/lib/format";
import { useConfig } from "@/lib/queries";
import { microphones } from "@/lib/voice/capture";
import { useVoiceDevice } from "@/lib/voice/device";
import { startDictation, stopDictation, useVoice, useVoiceAvailability, useVoiceStatus } from "@/lib/voice/dictation";
import type { Dictation } from "@/lib/voice/useDictation";
import { Card, ConfigError, SettingsPage, usePatchConfig } from "./SettingsView";

const LANGUAGES: [string, string][] = [["en", "English"], ["fr", "Français"], ["de", "Deutsch"], ["es", "Español"], ["it", "Italiano"], ["pt", "Português"], ["nl", "Nederlands"], ["pl", "Polski"], ["ru", "Русский"], ["ja", "日本語"], ["ko", "한국어"], ["zh", "中文"], ["hi", "हिन्दी"]];
const SILENCE: [number, string][] = [[5, "5 s"], [8, "8 s"], [15, "15 s"], [0, "Never"]];
const TEST_OWNER = "voice-test";
const STATE: Record<string, [BadgeTone, string]> = { ready: ["ok", "Ready"], used_up: ["attn", "Waiting"], refused: ["danger", "Sign in again"], no_login: ["attn", "No login"] };

function SourceRow({ s, index, count, onMove, onRemove }: { s: VoiceSource; index: number; count: number; onMove: (to: number) => void; onRemove: () => void }) {
  const [tone, label] = STATE[s.state] ?? ["neutral", s.state];
  const detail = s.state === "used_up" && s.resets_at ? `${s.detail} until ${when(s.resets_at * 1000)}, skipped` : s.detail;
  // The phone has no room for three 44 px buttons next to the badge: they go in ⋯.
  const phone = usePhone();
  return (
    <div className="flex min-h-12 items-center gap-3 border-b border-line-2 py-2 last:border-b-0">
      <span className="grid size-6 flex-none place-items-center rounded-full bg-sunk text-[12px] font-semibold text-ink-2">{index + 1}</span>
      <div className="grid min-w-0 flex-1 gap-0.5">
        <b className="truncate text-[13px] font-semibold">{s.harness} · {s.name}</b>
        <span className="truncate text-[12px] text-ink-3" title={detail}>{detail}</span>
      </div>
      <Badge tone={tone}>{label}</Badge>
      {phone ? <MoreMenu label={`${s.name} as a voice source`}>{(close) => <>
        <MenuItem icon={<Icon name="up" size={14} />} label="Move up" disabled={index === 0} onClick={() => { close(); onMove(index - 1); }} />
        <MenuItem icon={<Icon name="down" size={14} />} label="Move down" disabled={index === count - 1} onClick={() => { close(); onMove(index + 1); }} />
        <MenuItem icon={<Icon name="x" size={14} />} label="Remove from voice" onClick={() => { close(); onRemove(); }} />
      </>}</MoreMenu> : <span className="flex flex-none gap-0.5">
        <IconButton icon="up" label="Move up" disabled={index === 0} onClick={() => onMove(index - 1)} />
        <IconButton icon="down" label="Move down" disabled={index === count - 1} onClick={() => onMove(index + 1)} />
        <IconButton icon="x" label="Remove from voice" onClick={onRemove} />
      </span>}
    </div>
  );
}

type TestResult = { name: string; text: string; firstMs: number | null; error: string | null };

export function VoiceView() {
  const config = useConfig().data?.config;
  const status = useVoiceStatus();
  const { reason } = useVoiceAvailability();
  const patch = usePatchConfig();
  const qc = useQueryClient();
  const save = useSaveState();
  const device = useVoiceDevice();
  const [mics, setMics] = useState<MediaDeviceInfo[]>([]);
  const [tests, setTests] = useState<TestResult[] | null>(null);
  const testing = useVoice((s) => s.owner === TEST_OWNER && s.phase !== "idle");
  useEffect(() => { void microphones().then(setMics).catch(() => setMics([])); }, []);
  const voice: VoiceConfig = config?.voice ?? { enabled: true, profiles: [], language: "en", silence_stop_s: 8 };
  const sources = status.data?.sources ?? [];
  const used = sources.filter((s) => s.used);
  const addable = sources.filter((s) => !s.used && !["unsupported", "pay_as_you_go"].includes(s.state));
  const unsupported = sources.filter((s) => s.state === "unsupported" || s.state === "pay_as_you_go");
  const update = (next: Partial<VoiceConfig>) => void save.run(async () => {
    await patch({ voice: { ...voice, ...next } });
    await qc.invalidateQueries({ queryKey: ["voice"] });
  });
  const order = (ids: string[]) => update({ profiles: ids });
  const move = (from: number, to: number) => { const ids = used.map((s) => s.profile); const [id] = ids.splice(from, 1); ids.splice(to, 0, id); order(ids); };
  const runTest = async () => {
    const results: TestResult[] = [];
    setTests([]);
    for (const s of used.filter((x) => x.state === "ready")) {
      let text = "", first = 0;
      const t0 = Date.now();
      const unsubscribe = useVoice.subscribe((v) => { if (!first && v.owner === TEST_OWNER && v.interim) first = Date.now(); });
      await startDictation({ owner: TEST_OWNER, keyterms: [], profile: s.profile, insert: (p) => { if (!first) first = Date.now(); text = `${text} ${p}`.trim(); } }, { silenceS: 0 });
      if (useVoice.getState().phase !== "idle") {
        await new Promise((r) => setTimeout(r, 3000));
        await stopDictation();
      }
      unsubscribe();
      results.push({ name: `${s.harness} · ${s.name}`, text, firstMs: first ? first - t0 : null, error: useVoice.getState().error });
      setTests([...results]);
    }
  };
  const caption: Dictation = { id: TEST_OWNER, phase: "idle", active: false, shown: true, reason: null, source: null, toggle: () => {}, start: () => {}, stop: () => Promise.resolve(), stopAndSend: () => {}, cancel: () => {}, warm: () => {} };
  const meta = !config ? undefined : voice.enabled ? `On · ${used.length} source${used.length === 1 ? "" : "s"}` : "Off";
  return (
    <SettingsPage title="Voice" meta={meta} intro="Dictate in every message box: tap the mic left of Send, or hold Ctrl+Space. Your words are transcribed with your Claude subscription logins on Home; nothing is sent until you send it.">
      <ConfigError />
      {isDemo && <div className="rounded-card bg-attn-soft px-3 py-2 text-[12px] text-attn">The demo shows the settings only: voice needs your own Home and a Claude login.</div>}
      <Card title="Voice input">
        <SettingRow label="Voice input" hint="Adds a mic to every message box." scope="all" state={save.state} error={save.error}>
          <Choices label="Voice input"><Choice active={voice.enabled} onClick={() => update({ enabled: true })}>On</Choice><Choice active={!voice.enabled} onClick={() => update({ enabled: false })}>Off</Choice></Choices>
          {voice.enabled && reason && <span className="basis-full text-[12px] text-attn">{reason}</span>}
        </SettingRow>
      </Card>
      <Card title="Sources, in order" end={<button className="btn btn-sm" disabled={isDemo || testing || !voice.enabled || !used.some((s) => s.state === "ready")} title="Records 3 s with each ready source in order" onClick={() => void runTest()}><Icon name="mic" size={12} />{testing ? "Testing…" : "Test voice"}</button>}>
        {status.isLoading && !isDemo ? <Loading what="sources" /> : <div>
          {used.map((s, i) => <SourceRow key={s.profile} s={s} index={i} count={used.length} onMove={(to) => move(i, to)} onRemove={() => order(used.filter((x) => x.profile !== s.profile).map((x) => x.profile))} />)}
          {!used.length && <div className="py-2 text-[12px] text-ink-3">No source: voice uses Claude subscription profiles.</div>}
          {addable.map((s) => <div key={s.profile} className="flex min-h-12 items-center gap-3 border-b border-line-2 py-2 last:border-b-0">
            <span className="grid size-6 flex-none place-items-center text-[12px] text-ink-3">–</span>
            <div className="grid min-w-0 flex-1 gap-0.5"><b className="truncate text-[13px] font-semibold">{s.harness} · {s.name}</b><span className="truncate text-[12px] text-ink-3">Not used for voice · {s.detail}</span></div>
            <button className="btn btn-sm" onClick={() => order([...used.map((x) => x.profile), s.profile])}>Add</button>
          </div>)}
          {unsupported.length > 0 && <div className="flex min-h-12 items-center gap-3 py-2">
            <span className="grid size-6 flex-none place-items-center text-[12px] text-ink-3">–</span>
            <div className="grid min-w-0 flex-1 gap-0.5"><b className="truncate text-[13px] font-semibold text-ink-2">{[...new Set(unsupported.map((s) => s.state === "pay_as_you_go" ? "Pay-as-you-go" : s.harness))].join(", ")} profiles</b><span className="truncate text-[12px] text-ink-3" title={unsupported.map((s) => `${s.name}: ${s.detail}`).join("\n")}>Not supported for voice yet</span></div>
          </div>}
          <Link to="/settings/profiles" className="mt-1 inline-flex items-center gap-1 text-[12px] text-teal-ink hover:underline"><Icon name="plus" size={12} />Add a profile</Link>
        </div>}
        {testing && <VoiceCaption dictation={caption} className="" />}
        {tests && tests.length > 0 && <div className="grid gap-1 rounded-control bg-sunk px-3 py-2 text-[12px]">
          {tests.map((t) => <div key={t.name} className="grid gap-0.5"><b className="font-semibold">{t.name}{t.firstMs !== null && <span className="font-normal text-ink-3"> · first words after {(t.firstMs / 1000).toFixed(1)} s</span>}</b>
            <span className={t.error ? "text-danger-ink" : t.text ? "text-ink-2" : "text-ink-3"}>{t.error ?? (t.text ? `“${t.text}”` : "Heard nothing: say a few words while it records.")}</span></div>)}
        </div>}
      </Card>
      <Card title="Speech">
        <SettingRow label="Language" hint="The language you speak." scope="all">
          <select className="field-input w-auto" aria-label="Language" value={voice.language} onChange={(e) => update({ language: e.target.value })}>
            {LANGUAGES.map(([code, name]) => <option key={code} value={code}>{name}</option>)}
            {!LANGUAGES.some(([code]) => code === voice.language) && <option value={voice.language}>{voice.language}</option>}
          </select>
        </SettingRow>
        <SettingRow label="Stop after silence" hint="A 2 s countdown shows first; speaking cancels it. Dictation stops after 5 minutes anyway." scope="all">
          <Choices label="Stop after silence">{SILENCE.map(([s, label]) => <Choice key={s} active={voice.silence_stop_s === s} onClick={() => update({ silence_stop_s: s })}>{label}</Choice>)}</Choices>
        </SettingRow>
      </Card>
      <Card title="This device">
        <SettingRow label="Microphone" hint={mics.some((m) => m.label) ? undefined : "Names show once this site may use the microphone."} scope="device">
          <select className="field-input w-auto max-w-full" aria-label="Microphone" value={device.mic ?? ""} onChange={(e) => device.setMic(e.target.value || null)}>
            <option value="">Default microphone</option>
            {mics.filter((m) => m.deviceId && m.deviceId !== "default").map((m, i) => <option key={m.deviceId} value={m.deviceId}>{m.label || `Microphone ${i + 1}`}</option>)}
          </select>
        </SettingRow>
        <SettingRow label="Start and stop sounds" scope="device">
          <Choices label="Start and stop sounds"><Choice active={device.sounds} onClick={() => device.setSounds(true)}>On</Choice><Choice active={!device.sounds} onClick={() => device.setSounds(false)}>Off</Choice></Choices>
        </SettingRow>
      </Card>
    </SettingsPage>
  );
}
