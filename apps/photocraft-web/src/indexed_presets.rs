//! IndexedDB owns durable bytes; the engine sees only the bounded, Send memory bridge.
//! Each batch commits tips, groups and index together, and success means transaction completion.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use js_sys::{Array, Function, JsString, Promise, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, Event, EventTarget, IdbDatabase, IdbRequest, IdbTransactionMode};

use crate::preset_bridge::{ActionKeys, Batch, Bridge, MAX_RECORDS, WriteError, record_limit};
use photocraft_engine::preset_store::{ACTIONS_FILE, MAX_ACTIONS_BYTES, MAX_STORE_BYTES};

const DATABASE: &str = "photocraft.presets";
const FILES: &str = "files";
const TIMEOUT_MS: f64 = 10_000.0;

fn js(e: JsValue) -> String {
    if let Some(exception) = e.dyn_ref::<web_sys::DomException>() {
        format!("{}: {}", exception.name(), exception.message())
    } else {
        e.as_string().unwrap_or_else(|| format!("{e:?}"))
    }
}

/// Listeners are detached even when startup or a write times out.
type Listener = (EventTarget, &'static str, Closure<dyn FnMut(Event)>);
struct Listeners(Vec<Listener>);
impl Listeners {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn add(&mut self, target: &EventTarget, name: &'static str, f: impl FnMut(Event) + 'static) -> Result<(), JsValue> {
        let cb = Closure::<dyn FnMut(Event)>::new(f);
        target.add_event_listener_with_callback(name, cb.as_ref().unchecked_ref())?;
        self.0.push((target.clone(), name, cb));
        Ok(())
    }
}
impl Drop for Listeners {
    fn drop(&mut self) {
        for (target, name, cb) in &self.0 {
            let _ = target.remove_event_listener_with_callback(name, cb.as_ref().unchecked_ref());
        }
    }
}

struct Pending {
    promise: Promise,
    _listeners: Listeners,
}

fn request(request: &IdbRequest) -> Pending {
    let mut listeners = Listeners::new();
    let promise = Promise::new(&mut |resolve, reject| {
        let req = request.clone();
        let failed = reject.clone();
        if let Err(e) = listeners.add(request, "success", move |_| match req.result() {
            Ok(v) => {
                let _ = resolve.call1(&JsValue::NULL, &v);
            }
            Err(e) => {
                let _ = failed.call1(&JsValue::NULL, &e);
            }
        }) {
            let _ = reject.call1(&JsValue::NULL, &e);
        }
        let req = request.clone();
        let failed = reject.clone();
        if let Err(e) = listeners.add(request, "error", move |_| {
            let e = req.error().ok().flatten().map(JsValue::from).unwrap_or_else(|| JsValue::from_str("IndexedDB request failed"));
            let _ = failed.call1(&JsValue::NULL, &e);
        }) {
            let _ = reject.call1(&JsValue::NULL, &e);
        }
    });
    Pending { promise, _listeners: listeners }
}

/// A single deadline bounds the whole startup read, not ten seconds per record.
async fn before(promise: &Promise, deadline: f64) -> Result<JsValue, String> {
    let window = web_sys::window().ok_or("no window")?;
    let mut timer = None;
    let mut callback = None;
    let timeout = Promise::new(&mut |_, reject| {
        let cb = Closure::<dyn FnMut()>::new(move || {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("browser preset storage timed out"));
        });
        timer = Some(window.set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            (deadline - js_sys::Date::now()).clamp(0.0, TIMEOUT_MS) as i32,
        ));
        callback = Some(cb);
    });
    let id = timer.ok_or("could not start storage timeout")?.map_err(js)?;
    let result = JsFuture::from(Promise::race(&Array::of2(promise, &timeout))).await.map_err(js);
    window.clear_timeout_with_handle(id);
    drop(callback);
    result
}

async fn open(deadline: f64) -> Result<IdbDatabase, String> {
    let factory = web_sys::window().ok_or("no window")?.indexed_db().map_err(js)?.ok_or("IndexedDB unavailable")?;
    // Version 2 excludes old tabs that could still overwrite the paired Actions record with a Blob.
    let req = factory.open_with_u32(DATABASE, 2).map_err(js)?;
    let mut upgrade = Listeners::new();
    let upgrade_req = req.clone();
    upgrade
        .add(&req, "upgradeneeded", move |_| {
            let result = (|| -> Result<(), JsValue> {
                let db = upgrade_req.result()?.dyn_into::<IdbDatabase>()?;
                let tx = upgrade_req.transaction().ok_or_else(|| JsValue::from_str("No preset database upgrade transaction"))?;
                if tx.object_store(FILES).is_err() {
                    db.create_object_store(FILES)?;
                }
                Ok(())
            })();
            if result.is_err()
                && let Some(tx) = upgrade_req.transaction()
            {
                let _ = tx.abort();
            }
        })
        .map_err(js)?;
    let pending = request(&req);
    match before(&pending.promise, deadline).await {
        Ok(value) => value.dyn_into().map_err(|_| "invalid preset database".into()),
        Err(e) => {
            // An open request cannot be cancelled (another tab may block its upgrade). Keep
            // its handlers alive and close any late connection; never hydrate a live session.
            wasm_bindgen_futures::spawn_local(async move {
                if let Ok(value) = JsFuture::from(pending.promise.clone()).await
                    && let Ok(db) = value.dyn_into::<IdbDatabase>()
                {
                    db.close();
                }
                drop(pending);
                drop(upgrade);
            });
            Err(e)
        }
    }
}

pub async fn load() -> Result<(IdbDatabase, Bridge, Vec<String>), String> {
    let deadline = js_sys::Date::now() + TIMEOUT_MS;
    let db = open(deadline).await?;
    let result = read(&db, deadline).await;
    match result {
        Ok((bridge, warnings)) => Ok((db, bridge, warnings)),
        Err(e) => {
            db.close();
            Err(e)
        }
    }
}

async fn read(db: &IdbDatabase, deadline: f64) -> Result<(Bridge, Vec<String>), String> {
    let tx = db.transaction_with_str_and_mode(FILES, IdbTransactionMode::Readonly).map_err(js)?;
    let req = tx.object_store(FILES).map_err(js)?.open_cursor().map_err(js)?;
    let mut records = Vec::new();
    let mut warnings = Vec::new();
    let mut unreadable = Vec::new();
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut action_record = None;
    let mut action_issue = None;
    let mut next = request(&req);
    loop {
        let value = before(&next.promise, deadline).await?;
        if value.is_null() {
            break;
        }
        let cursor = value.dyn_into::<web_sys::IdbCursorWithValue>().map_err(|_| "invalid preset cursor")?;
        seen += 1;
        if seen > MAX_RECORDS {
            return Err(format!("browser preset record limit ({MAX_RECORDS}) exceeded"));
        }
        let key = cursor.key().map_err(js)?;
        // Inspect key length in JS before allocating its Rust String.
        let name = if key.is_string() && js_sys::JsString::from(key.clone()).length() <= 133 { key.as_string().unwrap_or_default() } else { String::new() };
        let stored = cursor.value().map_err(js)?;
        if name == ACTIONS_FILE && !stored.is_instance_of::<Blob>() {
            match paired_actions(&stored) {
                Ok(record) => action_record = Some(record),
                Err(error) => action_issue = Some(error),
            }
            next = request(&req);
            cursor.continue_().map_err(js)?;
            continue;
        }
        let blob = stored.dyn_into::<Blob>();
        let record = record_limit(&name).and_then(|limit| {
            let blob = blob.map_err(|_| "record is not a Blob".to_string())?;
            if blob.size() > limit as f64 {
                return Err("record is too large".into());
            }
            Ok(blob)
        });
        match record {
            Ok(blob) => {
                total = total.saturating_add(blob.size() as u64);
                if total > MAX_STORE_BYTES {
                    return Err("browser preset store exceeds size limit".into());
                }
                records.push((name, blob));
            }
            Err(e) => {
                // Leave an unreadable placeholder in the engine's file inventory. In
                // particular, a skipped group must protect its tip records from orphan GC.
                if record_limit(&name).is_ok() {
                    unreadable.push(name.clone());
                }
                if warnings.len() < 8 {
                    warnings.push(format!("Brush presets: {name} skipped: {e}"));
                }
            }
        }
        // Register the next callback before continuing the cursor. Blob reads happen after
        // enumeration, since awaiting them here would let this transaction become inactive.
        next = request(&req);
        cursor.continue_().map_err(js)?;
    }
    let bridge = Bridge::default();
    for name in unreadable {
        bridge.load(&name, Vec::new())?;
    }
    for (name, blob) in records {
        let result = before(&blob.array_buffer(), deadline).await.and_then(|buffer| bridge.load(&name, Uint8Array::new(&buffer).to_vec()));
        if let Err(error) = result {
            if name != ACTIONS_FILE {
                return Err(error);
            }
            action_issue = Some(error);
        }
    }
    if let Some((revision, definitions, keys)) = action_record {
        let result = async {
            // A paired record beyond the remaining definition budget leaves the ordinary presets usable.
            if total.saturating_add(definitions.as_ref().map_or(0, |blob| blob.size() as u64)) > MAX_STORE_BYTES {
                return Err("browser preset store exceeds size limit".into());
            }
            let buffer = before(&keys.array_buffer(), deadline).await?;
            let keys = ActionKeys::decode(&Uint8Array::new(&buffer).to_vec())?;
            if let Some(blob) = definitions {
                let buffer = before(&blob.array_buffer(), deadline).await?;
                bridge.load(ACTIONS_FILE, Uint8Array::new(&buffer).to_vec())?;
            }
            bridge.load_action_keys(revision, keys);
            Ok::<_, String>(())
        }
        .await;
        if let Err(error) = result {
            action_issue = Some(error);
        }
    }
    if let Some(error) = action_issue {
        // A placeholder tells the engine that this file exists and must be left unchanged.
        // It never enters a write batch, and the unrelated records loaded above remain usable.
        bridge.load(ACTIONS_FILE, Vec::new())?;
        warnings.push(format!("Browser Actions were not loaded: {error}"));
        bridge.quarantine_actions(error);
    }
    Ok((bridge, warnings))
}

/// A v2 Actions record has one generation, optional definition bytes and Action-only preferences.
fn paired_actions(value: &JsValue) -> Result<(u64, Option<Blob>, Blob), String> {
    let invalid = "Invalid paired browser Actions record";
    let row = value.dyn_ref::<Array>().filter(|row| row.length() == 3).ok_or(invalid)?;
    let revision = row.get(0);
    let generation = revision.is_string().then(|| JsString::from(revision.clone())).filter(|s| s.length() <= 20).ok_or(invalid)?;
    let text = generation.as_string().ok_or(invalid)?;
    let revision = text.parse::<u64>().ok().filter(|r| *r > 0 && r.to_string() == text).ok_or(invalid)?;
    let bytes = row.get(1);
    let definitions = if bytes.is_null() { None } else { Some(bytes.dyn_into::<Blob>().map_err(|_| invalid)?) };
    let keys = row.get(2).dyn_into::<Blob>().map_err(|_| invalid)?;
    if keys.size() > MAX_ACTIONS_BYTES as f64 || definitions.as_ref().is_some_and(|blob| blob.size() > MAX_ACTIONS_BYTES as f64) {
        return Err("Paired browser Actions record is too large".into());
    }
    Ok((revision, definitions, keys))
}

fn blob(bytes: &[u8]) -> Result<Blob, String> {
    Blob::new_with_u8_array_sequence(&Array::of1(&Uint8Array::from(bytes))).map_err(js)
}

pub async fn write(db: &IdbDatabase, batch: &Batch) -> Result<(), WriteError> {
    let tx = db.transaction_with_str_and_mode(FILES, IdbTransactionMode::Readwrite).map_err(js)?;
    let store = tx.object_store(FILES).map_err(js)?;
    let mut compare = Listeners::new();
    let compare_error = Rc::new(RefCell::new(None));
    let queued = (|| -> Result<(), String> {
        match batch {
            Batch::Files(files) => {
                for (name, bytes) in files {
                    let key = JsValue::from_str(name);
                    if let Some(bytes) = bytes {
                        store.put_with_key(blob(bytes)?.as_ref(), &key).map_err(js)?;
                    } else {
                        store.delete(&key).map_err(js)?;
                    }
                }
            }
            Batch::Actions(action) => {
                let keys = action.keys.encode()?;
                if keys.len() as u64 > MAX_ACTIONS_BYTES {
                    return Err("Browser Action shortcuts are too large to save".into());
                }
                let definitions = action.definitions.as_deref().map(blob).transpose()?.map_or(JsValue::NULL, JsValue::from);
                let keys = blob(&keys)?;
                let record = Array::of3(&JsValue::from_str(&action.revision.to_string()), &definitions, keys.as_ref());
                let base = action.base;
                let key = JsValue::from_str(ACTIONS_FILE);
                let request = store.get(&key).map_err(js)?;
                let read = request.clone();
                let store = store.clone();
                let tx = tx.clone();
                let compare_error = compare_error.clone();
                // Compare and queue synchronously while this isolated readwrite transaction is active.
                compare
                    .add(&request, "success", move |_| {
                        let outcome = (|| -> Result<(), WriteError> {
                            let current = read.result().map_err(js)?;
                            let revision = if current.is_undefined() || current.is_instance_of::<Blob>() {
                                0
                            } else {
                                paired_actions(&current).map_err(WriteError::Blocked)?.0
                            };
                            if revision != base {
                                return Err(WriteError::Stale("Actions changed in another tab.".into()));
                            }
                            store.put_with_key(&record, &key).map_err(js)?;
                            Ok(())
                        })();
                        if let Err(error) = outcome {
                            *compare_error.borrow_mut() = Some(error);
                            let _ = tx.abort();
                        }
                    })
                    .map_err(js)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = queued {
        return Err(if tx.abort().is_ok() { WriteError::Aborted(error) } else { WriteError::Uncertain(error) });
    }
    let mut listeners = Listeners::new();
    let terminal = Rc::new(Cell::new(None));
    let promise = Promise::new(&mut |resolve, reject| {
        let completed = terminal.clone();
        if let Err(e) = listeners.add(&tx, "complete", move |_| {
            completed.set(Some(true));
            let _ = resolve.call0(&JsValue::NULL);
        }) {
            let _ = reject.call1(&JsValue::NULL, &e);
        }
        let aborted = terminal.clone();
        let tx_for_error = tx.clone();
        let failed: Function = reject.clone();
        if let Err(e) = listeners.add(&tx, "abort", move |_| {
            aborted.set(Some(false));
            let e = tx_for_error.error().map(JsValue::from).unwrap_or_else(|| JsValue::from_str("preset transaction aborted"));
            let _ = failed.call1(&JsValue::NULL, &e);
        }) {
            let _ = reject.call1(&JsValue::NULL, &e);
        }
    });
    let outcome = before(&promise, js_sys::Date::now() + TIMEOUT_MS).await;
    if let Some(error) = compare_error.take() {
        // No put was queued on any comparison failure.
        return Err(error);
    }
    match terminal.get() {
        Some(true) => Ok(()),
        Some(false) => Err(WriteError::Aborted(outcome.err().unwrap_or_else(|| "Preset transaction aborted".into()))),
        None => {
            let _ = tx.abort();
            let error = outcome.err().unwrap_or_else(|| "No preset transaction result".into());
            let message = format!("Preset commit could not be confirmed: {error}");
            let message = if matches!(batch, Batch::Actions(_)) { format!("{message}. Reopen the page before saving Actions again.") } else { message };
            Err(WriteError::Uncertain(message))
        }
    }
}
