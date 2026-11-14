"use strict";
// ------------------------------------------------------------------------- //
// Authors and narrators: their books to tick and request together, and following them – a book that comes
// out later is then requested by itself (or only shown as new). The list of follows is Entdecken → Gefolgt.
// ------------------------------------------------------------------------- //
type PersonKind = "author" | "narrator";
interface Follow {
  id: number; kind: PersonKind; name: string; language: string; auto: boolean; since: number; checked: number | null;
  added: number; recent: string[]; fresh: string[]; note: string;
}
interface WorksAnswer { kind: PersonKind; name: string; language: string; follow: Follow | null; books: (Book & { requested?: string | null })[] }

/** Books per request of the window (the server takes at most this many at once). */
const PERSON_CHUNK = 60;

function personTitle(kind: PersonKind, name: string): string {
  return kind === "narrator" ? t("Gelesen von {name}", { name }) : t("Hörbücher von {name}", { name });
}

/** A name on a card that opens the person's books. */
function personLink(name: string, kind: PersonKind, b?: Book): HTMLButtonElement {
  return el("button", { type: "button", class: "linkish person-link",
    title: kind === "narrator" ? t("Alle Hörbücher, die {name} gelesen hat – und folgen", { name }) : t("Alle Hörbücher von {name} – und folgen", { name }),
    onclick: (e: Event) => { e.stopPropagation(); openPerson(kind, name, b ? bookLangCode(b) : ""); } }, name);
}

function personLinks(names: string[] | undefined, kind: PersonKind, b?: Book, max = 3): (HTMLElement | string)[] {
  const out: (HTMLElement | string)[] = [];
  (names || []).slice(0, max).forEach((n, i) => { if (i) out.push(", "); out.push(personLink(n, kind, b)); });
  if ((names || []).length > max) out.push(" ", t("u. a."));      // more names than fit: the person window lists them all
  return out;
}

/** The person window: follow (or not), and tick books to request. */
async function openPerson(kind: PersonKind, name: string, language: string): Promise<void> {
  const d = $<HTMLDialogElement>("#personDialog");
  const ok = $<HTMLButtonElement>("#personOk");
  const follow = $<HTMLButtonElement>("#personFollow");
  const auto = $<HTMLInputElement>("#personAuto");
  $("#personTitle").textContent = personTitle(kind, name);
  for (const r of document.querySelectorAll<HTMLInputElement>("#personLang input")) r.checked = r.value === language;
  d.returnValue = "";
  if (!d.open) d.showModal();
  let res: WorksAnswer | null = null;
  let rows: { v: Book; cb: HTMLInputElement; taken: string }[] = [];

  const count = (): void => {
    const n = rows.filter((r) => r.cb.checked).length;
    ok.disabled = !n;
    ok.textContent = plural(n, t("1 Hörbuch anfragen"), t("{n} Hörbücher anfragen", { n }));
  };
  const showFollow = (): void => {
    const f = res?.follow;
    follow.textContent = f ? t("Nicht mehr folgen") : t("Folgen");
    follow.classList.toggle("primary", !f);
    auto.checked = f ? f.auto : auto.checked;
    $("#personFollowState").textContent = !f ? t("Nicht gefolgt.")
      : f.auto ? t("Gefolgt seit {when}. Neue Hörbücher werden automatisch angefragt.", { when: fmtDay(f.since) })
        : t("Gefolgt seit {when}. Neue Hörbücher werden nur unter Entdecken → Gefolgt angezeigt.", { when: fmtDay(f.since) });
  };
  const load = async (): Promise<void> => {
    const lang = document.querySelector<HTMLInputElement>("#personLang input:checked")?.value || "";
    $("#personInfo").textContent = t("Lade die Hörbücher …");
    $("#personBooks").replaceChildren();
    ok.disabled = true;
    try { res = await api<WorksAnswer>("POST", "/api/follows/works", { kind, name, language: lang }); }
    catch (e) { $("#personInfo").textContent = errMsg(e); return; }
    showFollow();
    if (res.follow?.fresh.length) void api("PUT", `/api/follows/${res.follow.id}`, {}).catch(() => undefined);
    rows = res.books.map((v) => {
      const state = (v.requested || requests.keys[requestKey(v)]) as RequestStatus | undefined;
      const taken = v.owned ? t("In der Bibliothek") : state && state !== "declined" ? REQ_STATUS[state]?.() || state : "";
      return { v, cb: el("input", { type: "checkbox", disabled: !!taken }), taken };
    });
    $("#personBooks").replaceChildren(...rows.map(({ v, cb, taken }) => el("label", { class: "check series-vol" + (taken ? " is-taken" : "") }, cb,
      v.cover_url ? el("img", { class: "thumb", src: v.cover_url, alt: "", loading: "lazy" }) : coverArt(v.title, (v.authors || [])[0]),
      el("span", { class: "series-vol-text" },
        el("b", {}, v.title),
        el("span", { class: "muted small" }, [kind === "narrator" ? (v.authors || []).slice(0, 2).join(", ") : (v.narrators || []).slice(0, 2).join(", "),
          v.series ? `${v.series} ${v.series_position || ""}`.trim() : "", v.runtime_min ? fmtDuration(v.runtime_min * 60) : "",
          (v.release_date || "").slice(0, 4)].filter(Boolean).join(" · ")),
        taken ? el("span", { class: v.owned ? "rtag ok" : "rtag" }, taken) : editionNote(v)))));
    const free = rows.filter((r) => !r.taken).length;
    $("#personInfo").textContent = !rows.length ? t("Bei Audible nichts gefunden – Schreibweise prüfen oder eine andere Sprache wählen.")
      : plural(rows.length, t("1 Hörbuch bei Audible, das neueste zuerst."), t("{n} Hörbücher bei Audible, das neueste zuerst.", { n: rows.length })) + " "
        + (free ? t("Alles Angehakte geht als eine Anfrage los.") : t("Alle sind schon da oder angefragt."));
    rows.forEach((r) => r.cb.addEventListener("change", count));
    count();
  };

  $("#personAll").onclick = () => { rows.forEach((r) => { if (!r.taken) r.cb.checked = true; }); count(); };
  $("#personNone").onclick = () => { rows.forEach((r) => { r.cb.checked = false; }); count(); };
  for (const r of document.querySelectorAll<HTMLInputElement>("#personLang input")) r.onchange = () => { void load(); };
  follow.onclick = async () => {
    if (!res) return;
    try {
      if (res.follow) {
        await api("DELETE", `/api/follows/${res.follow.id}`);
        res.follow = null;
        toast(t("Du folgst {name} nicht mehr", { name }));
      } else {
        res.follow = await api<Follow>("POST", "/api/follows", { kind, name, language: res.language, auto: auto.checked });
        toast(auto.checked ? t("Du folgst {name} – neue Hörbücher werden automatisch angefragt", { name }) : t("Du folgst {name}", { name }));
      }
      showFollow();
      if (discover.section === "follows" && !$("#viewDiscover").hidden) void renderFollows();
    } catch (e) { toast(errMsg(e), true); }
  };
  auto.onchange = async () => {
    if (!res?.follow) return;
    try { await api("PUT", `/api/follows/${res.follow.id}`, { auto: auto.checked }); res.follow.auto = auto.checked; showFollow(); }
    catch (e) { toast(errMsg(e), true); auto.checked = !auto.checked; }
  };
  d.addEventListener("close", async () => {
    if (d.returnValue !== "ok" || !res) return;
    const books = rows.filter((r) => r.cb.checked).map((r) => r.v);
    let made = 0, status: RequestStatus = "searching";
    const skipped: string[] = [];
    try {
      for (let i = 0; i < books.length; i += PERSON_CHUNK) {
        const out = await api<{ status: RequestStatus; items: BookRequest[]; skipped: string[] }>("POST", "/api/requests/series",
          { series: { kind, title: name }, books: books.slice(i, i + PERSON_CHUNK) });
        for (const r of out.items) requests.keys[requestKey(r.book)] = r.status;
        made += out.items.length; status = out.status; skipped.push(...out.skipped);
      }
      toast((status === "pending" ? plural(made, t("1 Hörbuch angefragt – wartet auf Freigabe"), t("{n} Hörbücher angefragt – warten auf Freigabe", { n: made }))
        : plural(made, t("1 Hörbuch angefragt – Hörwerk sucht jetzt"), t("{n} Hörbücher angefragt – Hörwerk sucht jetzt", { n: made })))
        + (skipped.length ? " · " + t("übersprungen: {list}", { list: skipped.join(", ") }) : ""));
      loadRequests();
    } catch (e) { toast((made ? plural(made, t("1 Hörbuch angefragt"), t("{n} Hörbücher angefragt", { n: made })) + " · " : "") + errMsg(e), true); }
  }, { once: true });
  await load();
}

/** Entdecken → Gefolgt: everybody the account follows, with what came of it. */
async function renderFollows(): Promise<void> {
  const list = $("#discoverList");
  await loadList(list, async () => {
    const res = await api<{ follows: Follow[] }>("GET", "/api/follows");
    const name = el("input", { type: "search", placeholder: t("Name, z. B. Theodor Fontane"), "aria-label": t("Name") });
    const kind = el("select", { "aria-label": t("Autor oder Sprecher") },
      el("option", { value: "author" }, t("Autor")), el("option", { value: "narrator" }, t("Sprecher")));
    const add = el("form", { class: "follow-add" }, name, kind, el("button", { type: "submit", class: "btn primary" }, t("Hörbücher ansehen")));
    add.addEventListener("submit", (e) => {
      e.preventDefault();
      // the language of the page – a German household follows the German editions
      if (name.value.trim().length > 1) openPerson(kind.value as PersonKind, name.value.trim(), LANG.startsWith("en") ? "en" : "de");
    });
    const cards = res.follows.map((f) => el("li", { class: "follow-card" },
      el("div", { class: "s-body" },
        el("b", { class: "s-title" }, f.name),
        el("span", { class: "tag-row" }, el("span", { class: "tag" }, f.kind === "narrator" ? t("Sprecher") : t("Autor")),
          f.language ? el("span", { class: "lang-tag" + (f.language === "de" ? " de" : "") }, f.language === "de" ? t("Deutsch") : t("Englisch")) : null,
          f.fresh.length ? el("span", { class: "tag ok" }, plural(f.fresh.length, t("1 neues Hörbuch"), t("{n} neue Hörbücher", { n: f.fresh.length }))) : null),
        el("span", { class: "muted small" }, f.auto ? t("Neue Hörbücher werden automatisch angefragt") : t("Neue Hörbücher werden nur angezeigt")),
        f.added ? el("span", { class: "muted small" }, plural(f.added, t("Bisher 1 Hörbuch angefragt"), t("Bisher {n} Hörbücher angefragt", { n: f.added }))
          + (f.recent.length ? ": " + f.recent.slice(0, 3).join(", ") : "")) : null,
        f.fresh.length ? el("span", { class: "small" }, t("Neu: {titles}", { titles: f.fresh.slice(0, 3).join(", ") })) : null,
        f.note ? el("span", { class: "err small" }, f.note) : null,
        el("span", { class: "muted small" }, f.checked ? t("Zuletzt nachgesehen {when}", { when: fmtAgo(f.checked) }) : t("Wird heute zum ersten Mal nachgesehen"))),
      el("div", { class: "s-actions" },
        el("button", { type: "button", class: "btn small", onclick: () => openPerson(f.kind, f.name, f.language) }, t("Hörbücher ansehen")),
        el("button", { type: "button", class: "btn small ghost", onclick: async () => {
          try { await api("DELETE", `/api/follows/${f.id}`); toast(t("Du folgst {name} nicht mehr", { name: f.name })); void renderFollows(); }
          catch (e) { toast(errMsg(e), true); }
        } }, t("Nicht mehr folgen")))));
    list.replaceChildren(el("li", { class: "follow-intro" }, add,
      el("p", { class: "muted small" }, t("Einmal am Tag sieht Hörwerk bei Audible nach. Ein neues Hörbuch wird für dich angefragt – mit deinem Monatslimit und, wenn nötig, der Freigabe. Auf einer Karte führt ein Tipp auf den Namen von Autor oder Sprecher hierher."))),
    ...(cards.length ? cards : [el("li", { class: "empty" }, t("Du folgst noch niemandem."))]));
  }, { count: 3, kind: "card" });
}
