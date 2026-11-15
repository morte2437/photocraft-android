//! Release checks through `craft-update`, the library every Craft App shares. Only public release
//! metadata is requested.

use photocraft_engine::updates_cmds::{CheckFn, CheckResult, Release};

pub fn service() -> CheckFn {
    Box::new(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("update check".into())
            .spawn(move || {
                let result = check();
                if let Err(e) = &result {
                    log::warn!("Update check: {e}");
                }
                let _ = tx.send(result);
            })
            .map_err(|e| format!("Could not start update check: {e}"))?;
        Ok(rx)
    })
}

/// What the last self-update came to, once: `craft-update` removes its records as it reads them.
pub fn last_update() -> Option<Result<String, String>> {
    use craft_update::self_update::{Outcome, current_version, take_outcome};
    let app = &photocraft_ui_egui::UPDATE_APP;
    let paths = craft_update::Paths::for_app(app).ok()?;
    let version = photocraft_engine::build_info::VERSION;
    let running = current_version(app, version).map_or_else(|| version.to_string(), |v| v.to_string());
    match take_outcome(&paths, &running)? {
        Outcome::Installed(installed) => Some(Ok(installed.to_string())),
        Outcome::Failed(reason) => Some(Err(reason)),
    }
}

fn check() -> CheckResult {
    let http = craft_update::Http::new("PhotoCraft/Update")?;
    let found = craft_update::check_for_update(&photocraft_ui_egui::UPDATE_APP, &http, photocraft_engine::build_info::VERSION)?;
    Ok(found.map(|release| Release { version: release.tag.trim_start_matches(['v', 'V']).to_string(), url: release.html_url }))
}
