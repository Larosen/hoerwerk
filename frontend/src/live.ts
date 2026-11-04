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

let liveRetry = 0;
let liveLostTimer: number | undefined;
let liveWarned = false;

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
  es.addEventListener("open", () => {
    liveRetry = 0;
    clearTimeout(liveLostTimer);
    if (liveWarned) toast(t("Live-Verbindung wieder da"), "info");
    liveWarned = false;
    changed(true);
  });
  es.addEventListener("error", () => {
    changed(false);                       // until it is back the pages poll
    clearTimeout(liveLostTimer);
    // only a longer gap is worth a word (a restart of Hörwerk takes a few seconds)
    liveLostTimer = setTimeout(() => {
      if (!live.ok && !liveWarned) {
        liveWarned = true;
        toast(t("Live-Verbindung unterbrochen – die Seite aktualisiert sich langsamer, Hörwerk verbindet sich neu"), "warning");
      }
    }, 10_000);
    // the browser retries by itself – except after an error answer (e.g. 503 when busy): then we do, slower each time
    if (es.readyState === EventSource.CLOSED) {
      es.close();
      liveRetry = Math.min(liveRetry + 1, 6);
      setTimeout(startLive, Math.min(30_000, 1000 * 2 ** liveRetry));
    }
  });
}

// after all scripts registered their topics (a timer could fire between two scripts)
document.addEventListener("DOMContentLoaded", () => { if (IS_ADMIN) startLive(); });
