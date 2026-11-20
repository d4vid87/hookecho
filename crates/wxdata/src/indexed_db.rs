//! Browser-native promises for IndexedDB's two terminal event paths.
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{IdbDatabase, IdbRequest, IdbTransaction};

#[wasm_bindgen(inline_js = r#"
function result(request) {
    return new Promise((resolve, reject) => {
        const clear = () => { request.onsuccess = request.onerror = null; };
        request.onsuccess = () => { clear(); resolve(request.result); };
        request.onerror = () => { clear(); reject(request.error ?? new Error('IndexedDB request failed')); };
    });
}

export function indexedDbRequest(request) { return result(request); }

export function indexedDbOpen(factory, name, stores) {
    const request = factory.open(name, 1);
    request.onupgradeneeded = () => {
        for (const store of stores) {
            if (!request.result.objectStoreNames.contains(store)) request.result.createObjectStore(store);
        }
    };
    return result(request).finally(() => { request.onupgradeneeded = null; });
}

export function indexedDbCommitted(transaction, message) {
    const promise = new Promise((resolve, reject) => {
        const clear = () => { transaction.oncomplete = transaction.onabort = null; };
        transaction.oncomplete = () => { clear(); resolve(); };
        transaction.onabort = () => { clear(); reject(new Error(message)); };
    });
    // The failure may occur while the caller is still awaiting a queued request.
    promise.catch(() => {});
    return promise;
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = indexedDbRequest)]
    fn request_promise(request: IdbRequest) -> js_sys::Promise;
    #[wasm_bindgen(js_name = indexedDbOpen, catch)]
    fn open_promise(
        factory: web_sys::IdbFactory,
        name: &str,
        stores: js_sys::Array,
    ) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(js_name = indexedDbCommitted)]
    pub fn transaction_done(transaction: &IdbTransaction, message: &str) -> js_sys::Promise;
}

pub async fn await_request(request: IdbRequest) -> anyhow::Result<JsValue> {
    JsFuture::from(request_promise(request))
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}

pub async fn open(name: &str, stores: &[&str]) -> anyhow::Result<IdbDatabase> {
    let factory = web_sys::window()
        .and_then(|window| window.indexed_db().ok().flatten())
        .ok_or_else(|| anyhow::anyhow!("IndexedDB unavailable"))?;
    let stores = stores
        .iter()
        .map(|name| JsValue::from_str(name))
        .collect::<js_sys::Array>();
    let promise =
        open_promise(factory, name, stores).map_err(|error| anyhow::anyhow!("{error:?}"))?;
    JsFuture::from(promise)
        .await
        .and_then(|value| value.dyn_into())
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}
