// Live updates from the server (Server-Sent Events). Every topic pushes its full state when
// something changes; the old polling loops only run while this connection is down.
interface LiveTopics { audiobooks: AudiobooksStatus; library: LibraryStats }
type LiveHandler = (data: never) => void;

const live: { ok: boolean; handlers: Record<string, LiveHandler[]>; source: EventSource | null } = {
  ok: false, handlers: {}, source: null,
};

function onLive<K extends keyof LiveTopics>(topic: K, fn: (data: LiveTopics[K]) => void): void {
  (live.handlers[topic] ||= []).push(fn as LiveHandler);
}

function startLive(): void {
  const topics = Object.keys(live.handlers);
  if (!window.EventSource || !topics.length) return;
  const es = new EventSource("/api/events?topics=" + topics.join(","));
  live.source = es;
  for (const topic of topics) {
    es.addEventListener(topic, (e) => {
      let data: never;
      try { data = JSON.parse((e as MessageEvent<string>).data) as never; } catch { return; }
      for (const fn of live.handlers[topic]) fn(data);
    });
  }
  const changed = (ok: boolean): void => {
    if (live.ok === ok) return;
    live.ok = ok;
    document.dispatchEvent(new CustomEvent<boolean>("livechange", { detail: ok }));
  };
  es.addEventListener("open", () => changed(true));
  // the browser reconnects by itself (retry: 3000); until then the pages poll
  es.addEventListener("error", () => changed(false));
}

// after all scripts registered their topics (a timer could fire between two scripts)
document.addEventListener("DOMContentLoaded", () => { if (IS_ADMIN) startLive(); });
