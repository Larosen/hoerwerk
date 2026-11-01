"use strict";
// ------------------------------------------------------------------------- //
// Upload audiobooks from this computer into the inbox (e.g. ripped from CD).
// Files go one by one (streamed on the server), then the folders move into the inbox at once.
// ------------------------------------------------------------------------- //
interface UploadItem { file: File; path: string }

const upload = {
  items: [] as UploadItem[],
  id: "",
  busy: false,
  xhr: null as XMLHttpRequest | null,
};
const UPLOAD_EXT = /\.(mp3|m4a|m4b|mp4|aac|flac|ogg|oga|opus|wav|wma|jpe?g|png|webp|cue)$/i;
const UPLOAD_TEXT = /^(chapters\.txt|metadata\.ya?ml|metadata\.json|desc\.txt|reader\.txt)$/i;

function uploadable(path: string): boolean {
  const name = path.split("/").pop() || "";
  return !path.split("/").some((p) => p.startsWith(".") || p.startsWith("_")) && (UPLOAD_EXT.test(name) || UPLOAD_TEXT.test(name));
}

/** Folders dropped from the file manager: walk them (the browser only hands over entries). */
async function walkEntry(entry: FileSystemEntry, prefix: string, out: UploadItem[]): Promise<void> {
  if (entry.isFile) {
    const file = await new Promise<File>((res, rej) => (entry as FileSystemFileEntry).file(res, rej));
    out.push({ file, path: prefix + entry.name });
    return;
  }
  const reader = (entry as FileSystemDirectoryEntry).createReader();
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((res, rej) => reader.readEntries(res, rej));
    if (!batch.length) break;
    for (const child of batch) await walkEntry(child, prefix + entry.name + "/", out);
  }
}

function setUploadItems(items: UploadItem[]): void {
  const skipped = items.filter((i) => !uploadable(i.path)).length;
  upload.items = items.filter((i) => uploadable(i.path)).sort((a, b) => collator.compare(a.path, b.path));
  const loose = upload.items.some((i) => !i.path.includes("/"));
  $("#uploadNameRow").hidden = !loose;
  renderUploadList();
  $("#uploadNote").textContent = skipped ? t("{n} Dateien werden übersprungen (keine Audio- oder Cover-Dateien).", { n: skipped }) : "";
}

function renderUploadList(): void {
  const books = new Map<string, { files: number; bytes: number }>();
  for (const i of upload.items) {
    const top = i.path.includes("/") ? i.path.split("/")[0] : ($<HTMLInputElement>("#uploadName").value.trim() || t("(Name fehlt)"));
    const b = books.get(top) || { files: 0, bytes: 0 };
    b.files++; b.bytes += i.file.size;
    books.set(top, b);
  }
  $("#uploadList").replaceChildren(...[...books].map(([name, b]) => el("li", {},
    el("b", {}, name), el("span", { class: "muted small" }, t("{n} Dateien · {size}", { n: b.files, size: fmtSize(b.bytes) })))));
  $<HTMLButtonElement>("#uploadGo").disabled = !upload.items.length || upload.busy;
}

function sendFile(item: UploadItem, done: number, total: number): Promise<void> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    upload.xhr = xhr;
    xhr.open("PUT", `/api/upload/${upload.id}?path=${encodeURIComponent(item.path)}`);
    xhr.setRequestHeader("X-Requested-With", "Hoerwerk");
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    xhr.upload.onprogress = (e) => setUploadProgress((done + e.loaded) / total);
    xhr.onload = () => {
      if (xhr.status === 200) { resolve(); return; }
      let msg = `HTTP ${xhr.status}`;
      try { msg = JSON.parse(xhr.responseText).error || msg; } catch { /* plain text */ }
      reject(new Error(`${item.path}: ${msg}`));
    };
    xhr.onerror = () => reject(new Error(t("Verbindung abgebrochen")));
    xhr.onabort = () => reject(new Error(t("Upload abgebrochen")));
    xhr.send(item.file);
  });
}

function setUploadProgress(share: number): void {
  const bar = $("#uploadProgress");
  bar.hidden = false;
  (bar.firstElementChild as HTMLElement).style.width = `${Math.min(100, Math.round(share * 100))}%`;
}

async function startUpload(): Promise<void> {
  const name = $<HTMLInputElement>("#uploadName").value.trim().replace(/[\\/:*?"<>|]/g, " ");
  if (upload.items.some((i) => !i.path.includes("/")) && !name) {
    toast(t("Bitte einen Namen für das Hörbuch eintragen"), true);
    $<HTMLInputElement>("#uploadName").focus();
    return;
  }
  upload.busy = true;
  renderUploadList();
  const total = upload.items.reduce((n, i) => n + i.file.size, 0) || 1;
  let done = 0;
  try {
    upload.id = (await api<{ id: string }>("POST", "/api/upload")).id;
    for (const [n, item] of upload.items.entries()) {
      $("#uploadNote").textContent = t("Datei {n} von {total}: {name}", { n: n + 1, total: upload.items.length, name: item.path.split("/").pop() || "" });
      await sendFile({ ...item, path: item.path.includes("/") ? item.path : `${name}/${item.path}` }, done, total);
      done += item.file.size;
    }
    const res = await api<{ names: string[] }>("POST", `/api/upload/${upload.id}/finish`);
    toast(t("Hochgeladen – wird jetzt verarbeitet: {names}", { names: res.names.join(", ") }));
    upload.id = "";
    $<HTMLDialogElement>("#uploadDialog").close();
    setTimeout(() => pollBooks(true), 800);
  } catch (e) {
    toast(errMsg(e), true);
    $("#uploadNote").textContent = errMsg(e);
    if (upload.id) api("DELETE", `/api/upload/${upload.id}`).catch(() => { /* gone anyway */ });
    upload.id = "";
  } finally {
    upload.busy = false;
    upload.xhr = null;
    renderUploadList();
  }
}

function openUpload(): void {
  setUploadItems([]);
  $("#uploadProgress").hidden = true;
  $<HTMLInputElement>("#uploadName").value = "";
  $<HTMLDialogElement>("#uploadDialog").showModal();
}

function initUpload(): void {
  if (!IS_ADMIN) return;
  $("#uploadBtn").addEventListener("click", openUpload);
  const folder = $<HTMLInputElement>("#uploadFolder");
  folder.addEventListener("change", () => {
    setUploadItems([...(folder.files || [])].map((f) => ({ file: f, path: (f as File & { webkitRelativePath: string }).webkitRelativePath || f.name })));
    folder.value = "";
  });
  const files = $<HTMLInputElement>("#uploadFiles");
  files.addEventListener("change", () => {
    setUploadItems([...(files.files || [])].map((f) => ({ file: f, path: f.name })));
    files.value = "";
  });
  $("#uploadName").addEventListener("input", renderUploadList);
  const drop = $("#uploadDrop");
  drop.addEventListener("dragover", (e) => { e.preventDefault(); drop.classList.add("over"); });
  drop.addEventListener("dragleave", () => drop.classList.remove("over"));
  drop.addEventListener("drop", async (e) => {
    e.preventDefault();
    drop.classList.remove("over");
    const out: UploadItem[] = [];
    const entries = [...(e.dataTransfer?.items || [])].map((i) => i.webkitGetAsEntry()).filter((x): x is FileSystemEntry => !!x);
    try { for (const entry of entries) await walkEntry(entry, "", out); } catch (err) { toast(errMsg(err), true); }
    setUploadItems(out);
  });
  $("#uploadGo").addEventListener("click", startUpload);
  $("#uploadCancel").addEventListener("click", () => {
    if (upload.busy && upload.xhr) upload.xhr.abort();
    else $<HTMLDialogElement>("#uploadDialog").close();
  });
}

initUpload();
