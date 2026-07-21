import init, { EmLogN } from "../pkg/em_log_n_wasm.js";

const DB_VERSION = 1;
const ROWS = "rows";
const SEP = "\u001f";

export async function openEmLogNBrowserShard({ spec, dbName = "em-log-n", wasmUrl } = {}) {
  if (!spec || !spec.domain) throw new Error("spec.domain is required");
  await init(wasmUrl);
  const db = await openDb(dbName);
  const shard = new EmLogN(JSON.stringify(spec));
  const rows = await readDomainRows(db, spec.domain);
  shard.load_rows_json(JSON.stringify(rows));

  return {
    domain: spec.domain,
    wasm: shard,
    async put(row) {
      const stored = jsObject(shard.put_json(JSON.stringify(row)));
      await putStoredRow(db, spec.domain, stored);
      return stored;
    },
    async deleteKey(key) {
      await deleteStoredRow(db, spec.domain, key);
      return shard.delete_key(key);
    },
    async applyPatch(patch) {
      const ops = Array.isArray(patch) ? patch : patch?.ops;
      if (!Array.isArray(ops)) throw new Error("patch must be an array or { ops: [...] }");
      for (const op of ops) {
        if (op.op === "upsert") {
          const row = op.row ?? op;
          await this.put({
            ts_nanos: row.ts_nanos,
            tiebreaker: row.tiebreaker,
            payload: row.payload,
            vectors: row.vectors,
          });
        } else if (op.op === "delete") {
          await this.deleteKey(op.key);
        } else {
          throw new Error(`unknown em-log-n patch op: ${op.op}`);
        }
      }
      return shard.len();
    },
    async applyPatchJsonl(rowsJsonl) {
      const ops = rowsJsonl
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter(Boolean)
        .map((line) => JSON.parse(line));
      return this.applyPatch(ops);
    },
    scan(limit = 50) {
      return jsObject(shard.scan(limit));
    },
    ann(indexName, query, k = 10) {
      return jsObject(shard.ann(indexName, JSON.stringify(query), k));
    },
    annInWindow(indexName, query, k, tLo, tHi) {
      return jsObject(shard.ann_in_window(indexName, JSON.stringify(query), k, String(tLo), String(tHi)));
    },
    async reload() {
      shard.clear();
      const fresh = await readDomainRows(db, spec.domain);
      shard.load_rows_json(JSON.stringify(fresh));
      return fresh.length;
    },
    async clear() {
      await clearDomainRows(db, spec.domain);
      shard.clear();
    },
    len() {
      return shard.len();
    },
  };
}

function jsObject(value) {
  if (value instanceof Map) {
    return Object.fromEntries([...value].map(([key, nested]) => [key, jsObject(nested)]));
  }
  if (Array.isArray(value)) return value.map(jsObject);
  return value;
}

function rowId(domain, key) {
  return `${domain}${SEP}${key}`;
}

function openDb(name) {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(name, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(ROWS)) {
        const store = db.createObjectStore(ROWS, { keyPath: "id" });
        store.createIndex("domain", "domain", { unique: false });
      }
    };
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
  });
}

function txDone(tx) {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(tx.error);
    tx.onerror = () => reject(tx.error);
  });
}

async function putStoredRow(db, domain, row) {
  const tx = db.transaction(ROWS, "readwrite");
  tx.objectStore(ROWS).put({ id: rowId(domain, row.key), domain, row });
  await txDone(tx);
}

async function deleteStoredRow(db, domain, key) {
  const tx = db.transaction(ROWS, "readwrite");
  tx.objectStore(ROWS).delete(rowId(domain, key));
  await txDone(tx);
}

function readDomainRows(db, domain) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction(ROWS, "readonly");
    const prefix = `${domain}${SEP}`;
    const range = IDBKeyRange.bound(prefix, `${prefix}\uffff`);
    const request = tx.objectStore(ROWS).openCursor(range);
    const rows = [];
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const cursor = request.result;
      if (!cursor) {
        resolve(rows);
        return;
      }
      rows.push(cursor.value.row);
      cursor.continue();
    };
  });
}

function clearDomainRows(db, domain) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction(ROWS, "readwrite");
    const prefix = `${domain}${SEP}`;
    const range = IDBKeyRange.bound(prefix, `${prefix}\uffff`);
    const request = tx.objectStore(ROWS).openCursor(range);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const cursor = request.result;
      if (cursor) {
        cursor.delete();
        cursor.continue();
      }
    };
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(tx.error);
    tx.onerror = () => reject(tx.error);
  });
}
