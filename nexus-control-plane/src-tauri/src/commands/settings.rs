use nexus_core::db::DbState;
use nexus_core::services::settings as settings_svc;

#[tauri::command]
pub fn get_setting(key: String, state: tauri::State<'_, DbState>) -> Result<Option<String>, String> {
    settings_svc::get_setting(&state, &key)
}

#[tauri::command]
pub fn set_setting(key: String, value: String, state: tauri::State<'_, DbState>) -> Result<(), String> {
    settings_svc::set_setting(&state, &key, &value)
}
