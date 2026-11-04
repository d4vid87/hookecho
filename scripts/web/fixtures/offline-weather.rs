use wasm_bindgen::prelude::*;
#[wasm_bindgen]
pub async fn save_pack(date: String, names: String, satellite: String) -> Result<String, JsValue> {
    let _guard = hookecho::webcache::begin_save().ok_or_else(|| JsValue::from_str("busy"))?;
    let names: Vec<String> = serde_json::from_str(&names).unwrap();
    let sat: Vec<String> = serde_json::from_str(&satellite).unwrap();
    let pack = hookecho::webcache::save_pack("KTLX", &date,
        names.into_iter().map(|name| (name, vec![1u8,2,3])).collect(), sat,
        vec![], Default::default(), vec![]).await.map_err(|error| JsValue::from_str(&error.to_string()))?;
    Ok(serde_json::to_string(&pack).unwrap())
}
#[wasm_bindgen]
pub async fn delete_pack(date: String) {
    hookecho::webcache::remove(format!("KTLX-{date}")).await;
}
#[wasm_bindgen]
pub async fn seed_object(key: String) -> Result<(), JsValue> {
    wxdata::object_cache::put("satellite", &key, &[9,8,7], chrono::Utc::now()).await
        .map_err(|error| JsValue::from_str(&error.to_string()))
}
#[wasm_bindgen]
pub async fn ready() -> String {
    serde_json::to_string(&hookecho::webcache::packs().await.iter().map(|pack| pack.available).collect::<Vec<_>>()).unwrap()
}

#[wasm_bindgen]
pub fn cancel_save() { hookecho::webcache::cancel_save(); }
