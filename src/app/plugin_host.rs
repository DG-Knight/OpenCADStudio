// HostSession — plugin-facing API implemented inside `app` (private field access).

use std::any::Any;

use acadrust::tables::AppId;
use acadrust::xdata::ExtendedDataRecord;
use ocs_plugin_api::host::{CadDocument, EntityType, Handle, HostApi, HostSettingValue};
use ocs_plugin_api::shm::{DocumentSnapshotStore, DocumentViewData};

#[cfg(not(target_arch = "wasm32"))]
use crate::plugin::v4_support;
use super::OpenCADStudio;

/// Session adapter: one active document tab, command line, undo.
pub(crate) struct HostSession<'a> {
    app: &'a mut OpenCADStudio,
    tab: usize,
    doc_store: Option<DocumentSnapshotStore<DocumentViewData>>,
}

impl<'a> HostSession<'a> {
    pub(crate) fn new(app: &'a mut OpenCADStudio, tab: usize) -> Self {
        Self {
            app,
            tab,
            doc_store: None,
        }
    }

    pub fn tab_index(&self) -> usize {
        self.tab
    }

    pub fn tab_id(&self) -> u64 {
        self.app.tabs[self.tab].id
    }

    pub fn document_path(&self, tab_id: u64) -> Option<std::path::PathBuf> {
        self.app
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.current_path.clone())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn document_view_v4(&mut self, tab_id: u64) -> Option<ocs_plugin_api::shm::DocumentViewInfo> {
        if tab_id != self.tab_id() {
            return None;
        }
        v4_support::open_document_view_v4(tab_id, self.document())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn close_document_view_v4(&mut self, tab_id: u64) {
        if tab_id == self.tab_id() {
            v4_support::close_document_view_v4(tab_id);
        }
    }

    pub fn document(&self) -> &CadDocument {
        &self.app.tabs[self.tab].scene.document
    }

    pub fn document_mut(&mut self) -> &mut CadDocument {
        &mut self.app.tabs[self.tab].scene.document
    }

    pub fn document_view(&mut self) -> Option<ocs_plugin_api::shm::DocumentViewInfo> {
        if self.doc_store.is_none() {
            let mut store = DocumentSnapshotStore::<DocumentViewData>::new(self.tab as u64, 8 * 1024 * 1024).ok()?;
            store.publish(&self.document().into()).ok()?;
            self.doc_store = Some(store);
        }
        let store = self.doc_store.as_ref()?;
        Some(ocs_plugin_api::shm::DocumentViewInfo {
            path: store.path().to_string_lossy().to_string(),
            version: store.version(),
        })
    }

    fn publish_document_view(&mut self) {
        let doc = &self.app.tabs[self.tab].scene.document;
        // Only publish views that have been opened by a consumer. V3 is lazily
        // created by document_view(); V4 is tracked per-tab by the V4 manager.
        if let Some(store) = self.doc_store.as_mut() {
            if let Err(e) = store.publish(&doc.into()) {
                eprintln!(
                    "[host] failed to publish document view for tab {}: {e}",
                    self.tab
                );
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let key = (self.tab_id(), self.app.tabs[self.tab].scene.geometry_epoch);
            if self.app.last_plugin_document != Some(key) {
                v4_support::publish_drawing_changed(key.0, key.1);
            }
            v4_support::publish_document_view_v4(self.tab_id(), doc);
            self.app.last_plugin_document =
                Some((self.tab_id(), self.app.tabs[self.tab].scene.geometry_epoch));
        }
    }

    pub fn add_entity(&mut self, entity: EntityType) -> Handle {
        let handle = self.app.tabs[self.tab].scene.add_entity(entity);
        self.publish_document_view();
        handle
    }

    pub fn add_entities(&mut self, entities: Vec<EntityType>) -> Vec<Handle> {
        let handles = self.app.tabs[self.tab].scene.add_entities(entities);
        self.publish_document_view();
        handles
    }

    pub fn bump_geometry(&mut self) {
        self.app.tabs[self.tab].scene.bump_geometry();
    }

    /// Replace the entity carrying `entity`'s handle in place, refreshing the
    /// scene's derived caches. Returns `false` when no entity has that handle.
    pub fn update_entity(&mut self, entity: EntityType) -> bool {
        let ok = self.app.tabs[self.tab].scene.update_entity(entity);
        if ok {
            self.publish_document_view();
        }
        ok
    }

    pub fn update_entities_transaction(
        &mut self,
        label: &str,
        mut entities: Vec<EntityType>,
    ) -> Result<(), String> {
        use std::collections::HashSet;
        if label.trim().is_empty() { return Err("transaction label is empty".into()); }
        if entities.is_empty() { return Ok(()); }
        let mut seen = HashSet::new();
        for entity in &mut entities {
            let handle = entity.common().handle;
            if handle.is_null() || !seen.insert(handle) {
                return Err(format!("null or duplicate entity handle: {handle:?}"));
            }
            let Some(existing) = self.document().get_entity(handle) else {
                return Err(format!("entity {handle:?} does not exist"));
            };
            if std::mem::discriminant(existing) != std::mem::discriminant(entity) {
                return Err(format!("entity {handle:?} changes kind"));
            }
            if existing.common().owner_handle != entity.common().owner_handle {
                return Err(format!("entity {handle:?} changes owner"));
            }
            ocs_plugin_api::entity_coverage::bind_canvas_entity_references(
                self.document(), entity,
            ).map_err(|error| format!("entity {handle:?}: {error}"))?;
            ocs_plugin_api::entity_coverage::validate_entity_mutation(existing, entity)
                .map_err(|error| format!("entity {handle:?}: {error}"))?;
            if self.app.tabs[self.tab].scene.is_layer_locked(handle) {
                return Err(format!("entity {handle:?} is on a locked layer"));
            }
        }
        self.push_undo(label);
        for entity in entities {
            // Validation above makes this infallible while the session owns
            // the document exclusively.
            assert!(self.app.tabs[self.tab].scene.update_entity(entity));
        }
        self.set_dirty();
        self.publish_document_view();
        Ok(())
    }

    pub fn selection(&self) -> Vec<Handle> {
        self.app.tabs[self.tab].scene.selected_handles_in_order()
    }

    pub fn set_selection(&mut self, handles: &[Handle]) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for handle in handles {
            if !seen.insert(*handle) {
                return Err(format!("duplicate selected entity: {handle:?}"));
            }
            if self.document().get_entity(*handle).is_none() {
                return Err(format!("entity {handle:?} does not exist"));
            }
        }
        if self.selection() == handles { return Ok(()); }
        let scene = &mut self.app.tabs[self.tab].scene;
        scene.replace_selection_exact(handles);
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.tab == self.app.active_tab {
                self.app.last_plugin_selection = Some((self.tab_id(), self.app.tabs[self.tab].scene.selection_fingerprint()));
            }
            crate::plugin::v4_support::publish_selection_changed_v4(self.tab_id(), self.selection());
        }
        Ok(())
    }

    /// Delete the entity with `handle`, keeping the scene's render caches in
    /// sync. Returns `false` when the entity is absent or on a locked layer
    /// (which `erase_entities` refuses to remove).
    pub fn remove_entity(&mut self, handle: Handle) -> bool {
        if self.document().get_entity(handle).is_none() {
            return false;
        }
        self.app.tabs[self.tab].scene.erase_entities(&[handle]);
        let removed = self.document().get_entity(handle).is_none();
        if removed {
            self.publish_document_view();
        }
        removed
    }

    // ── XDATA convenience ──────────────────────────────────────────────────
    // Plugins persist domain data as XDATA on plain entities so it round-trips
    // through DWG/DXF. These wrap the `acadrust::xdata` API keyed by entity
    // handle and keep the APPID table in sync.

    /// Read the XDATA record for `app_name` attached to entity `handle`, if any.
    pub fn read_record(&self, handle: Handle, app_name: &str) -> Option<&ExtendedDataRecord> {
        self.document()
            .get_entity(handle)?
            .common()
            .extended_data
            .get_record(app_name)
    }

    /// Attach `record` to entity `handle`, replacing any existing record for the
    /// same application. Registers the application in the APPID table when
    /// missing so the file stays valid for other CAD apps. Returns `false` when
    /// the entity does not exist.
    pub fn write_record(&mut self, handle: Handle, record: ExtendedDataRecord) -> bool {
        if self.app.tabs[self.tab].scene.is_layer_locked(handle) {
            return false;
        }
        let app = record.application_name.clone();
        self.ensure_app_id(&app);
        let app_handle = self.document().app_ids.get(&app).map(|a| a.handle.value());
        let Some(entity) = self.document_mut().get_entity_mut(handle) else {
            return false;
        };
        let xd = &mut entity.common_mut().extended_data;
        // Drop any existing record for this app, then append the new one.
        let kept: Vec<_> = xd
            .records()
            .iter()
            .filter(|r| r.application_name != app)
            .cloned()
            .collect();
        xd.clear();
        for r in kept {
            xd.add_record(r);
        }
        xd.add_record(record);
        // Drop stale verbatim EED for this app so the fresh record — not the
        // pre-edit bytes captured on a prior read — wins on the next save.
        // Otherwise a plugin's edit made after a save/reopen (which registered
        // the app in `raw_dwg_eed`) would not persist.
        if let Some(ah) = app_handle {
            xd.raw_dwg_eed.retain(|(a, _)| *a != ah);
        }
        self.bump_geometry();
        self.set_dirty();
        self.publish_document_view();
        true
    }

    /// Remove the XDATA record for `app_name` from entity `handle`. Returns
    /// `true` when a record was actually removed.
    pub fn remove_record(&mut self, handle: Handle, app_name: &str) -> bool {
        if self.app.tabs[self.tab].scene.is_layer_locked(handle) {
            return false;
        }
        let app_handle = self.document().app_ids.get(app_name).map(|a| a.handle.value());
        let Some(entity) = self.document_mut().get_entity_mut(handle) else {
            return false;
        };
        let xd = &mut entity.common_mut().extended_data;
        let kept: Vec<_> = xd
            .records()
            .iter()
            .filter(|r| r.application_name != app_name)
            .cloned()
            .collect();
        let removed_record = kept.len() != xd.records().len();
        // Also drop verbatim EED for this app so the removal persists across a
        // save (a record read back from DWG lives in `raw_dwg_eed`).
        let removed_raw = app_handle
            .map(|ah| xd.raw_dwg_eed.iter().any(|(a, _)| *a == ah))
            .unwrap_or(false);
        if !removed_record && !removed_raw {
            return false;
        }
        xd.clear();
        for r in kept {
            xd.add_record(r);
        }
        if let Some(ah) = app_handle {
            xd.raw_dwg_eed.retain(|(a, _)| *a != ah);
        }
        self.bump_geometry();
        self.set_dirty();
        self.publish_document_view();
        true
    }

    /// Register `name` in the APPID table if it is not already present, so XDATA
    /// written under it survives a DWG/DXF round-trip. The entry is given a real
    /// handle — a null-handle APPID is written as handle 0, which the DWG EED
    /// reference then can't resolve, so the XDATA would be dropped on reopen.
    fn ensure_app_id(&mut self, name: &str) {
        let doc = self.document_mut();
        if !doc.app_ids.contains(name) {
            let mut app = AppId::new(name);
            app.handle = doc.allocate_handle();
            let _ = doc.app_ids.add(app);
        }
    }

    pub fn push_undo(&mut self, label: &str) {
        self.app.push_undo_snapshot(self.tab, label);
    }

    pub fn set_dirty(&mut self) {
        self.app.tabs[self.tab].dirty = true;
    }

    pub fn system_variable(&self, name: &str) -> Option<HostSettingValue> {
        match name.to_ascii_uppercase().as_str() {
            "CLAYER" => Some(HostSettingValue::Text(
                self.document().header.current_layer_name.clone(),
            )),
            "SNAPANG" => Some(HostSettingValue::Number(self.app.snap_angle_deg as f64)),
            _ => None,
        }
    }

    pub fn set_system_variable(
        &mut self,
        name: &str,
        value: HostSettingValue,
    ) -> Result<HostSettingValue, String> {
        match (name.to_ascii_uppercase().as_str(), value) {
            ("CLAYER", HostSettingValue::Text(layer)) => {
                self.app.set_current_layer_name(self.tab, &layer)?;
                Ok(HostSettingValue::Text(layer))
            }
            ("SNAPANG", HostSettingValue::Number(angle)) if (angle as f32).is_finite() => {
                self.app.snap_angle_deg = (angle as f32).rem_euclid(360.0);
                Ok(HostSettingValue::Number(self.app.snap_angle_deg as f64))
            }
            ("SNAPANG", HostSettingValue::Number(_)) => {
                Err("SNAPANG: finite number required".to_owned())
            }
            ("CLAYER", _) | ("SNAPANG", _) => Err(format!(
                "{}: wrong value type",
                name.to_ascii_uppercase()
            )),
            _ => Err(format!("unsupported system variable {name:?}")),
        }
    }

    pub fn push_info(&mut self, msg: &str) {
        self.app.command_line.push_info(msg);
    }

    pub fn push_output(&mut self, msg: &str) {
        self.app.command_line.push_output(msg);
    }

    pub fn push_error(&mut self, msg: &str) {
        self.app.command_line.push_error(msg);
    }
}

/// The stable contract a plugin's `dispatch` sees. Each method forwards to the
/// inherent `HostSession` method of the same name (inherent methods take
/// resolution priority, so this is plain delegation, not recursion). The
/// per-tab plugin-state accessors expose the raw `Any` box; the typed
/// `ocs_plugin_api::host::plugin_state*` helpers wrap them.
impl HostApi for HostSession<'_> {
    fn tab_index(&self) -> usize {
        self.tab_index()
    }
    fn document(&self) -> &CadDocument {
        self.document()
    }
    fn document_mut(&mut self) -> &mut CadDocument {
        self.document_mut()
    }
    fn add_entity(&mut self, entity: EntityType) -> Handle {
        self.add_entity(entity)
    }
    fn add_entities(&mut self, entities: Vec<EntityType>) -> Vec<Handle> {
        self.add_entities(entities)
    }
    fn update_entity(&mut self, entity: EntityType) -> bool {
        self.update_entity(entity)
    }
    fn update_entities_transaction(&mut self, label: &str, entities: Vec<EntityType>) -> Result<(), String> {
        self.update_entities_transaction(label, entities)
    }
    fn selection(&self) -> Vec<Handle> { self.selection() }
    fn set_selection(&mut self, handles: &[Handle]) -> Result<(), String> { self.set_selection(handles) }
    fn remove_entity(&mut self, handle: Handle) -> bool {
        self.remove_entity(handle)
    }
    fn bump_geometry(&mut self) {
        self.bump_geometry()
    }
    fn read_record(&self, handle: Handle, app_name: &str) -> Option<&ExtendedDataRecord> {
        self.read_record(handle, app_name)
    }
    fn write_record(&mut self, handle: Handle, record: ExtendedDataRecord) -> bool {
        self.write_record(handle, record)
    }
    fn remove_record(&mut self, handle: Handle, app_name: &str) -> bool {
        self.remove_record(handle, app_name)
    }
    fn push_undo(&mut self, label: &str) {
        self.push_undo(label)
    }
    fn set_dirty(&mut self) {
        self.set_dirty()
    }
    fn push_info(&mut self, msg: &str) {
        self.push_info(msg)
    }
    fn push_output(&mut self, msg: &str) {
        self.push_output(msg)
    }
    fn push_error(&mut self, msg: &str) {
        self.push_error(msg)
    }
    fn start_interactive(&mut self, command: Box<dyn ocs_plugin_api::host::InteractiveCommand>) {
        self.app.tabs[self.tab].active_cmd =
            Some(Box::new(PluginInteractiveAdapter { inner: command }));
    }
    fn plugin_state_any(&self, plugin_id: &str) -> Option<&(dyn Any + Send + Sync)> {
        self.app.tabs[self.tab]
            .plugin_state
            .get(plugin_id)
            .map(|b| b.as_ref())
    }
    fn plugin_state_any_mut(&mut self, plugin_id: &str) -> Option<&mut (dyn Any + Send + Sync)> {
        self.app.tabs[self.tab]
            .plugin_state
            .get_mut(plugin_id)
            .map(|b| b.as_mut())
    }
    fn ensure_plugin_state_any(
        &mut self,
        plugin_id: &'static str,
        init: &mut dyn FnMut() -> Box<dyn Any + Send + Sync>,
    ) -> &mut (dyn Any + Send + Sync) {
        self.app.tabs[self.tab]
            .plugin_state
            .entry(plugin_id)
            .or_insert_with(|| init())
            .as_mut()
    }
    fn document_reader(&self) -> Box<dyn ocs_plugin_api::host::DocumentReader + '_> {
        Box::new(ocs_plugin_api::host::CadDocumentReader(self.document()))
    }
    fn document_view(&mut self) -> Option<ocs_plugin_api::shm::DocumentViewInfo> {
        self.document_view()
    }
    fn tab_id(&self) -> u64 {
        self.tab_id()
    }
    fn document_path(&self, tab_id: u64) -> Option<std::path::PathBuf> {
        self.document_path(tab_id)
    }
    fn system_variable(&self, name: &str) -> Option<HostSettingValue> {
        self.system_variable(name)
    }
    fn set_system_variable(
        &mut self,
        name: &str,
        value: HostSettingValue,
    ) -> Result<HostSettingValue, String> {
        self.set_system_variable(name, value)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn document_view_v4(&mut self, tab_id: u64) -> Option<ocs_plugin_api::shm::DocumentViewInfo> {
        self.document_view_v4(tab_id)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn close_document_view_v4(&mut self, tab_id: u64) {
        self.close_document_view_v4(tab_id)
    }
}

/// Bridges a plugin's [`InteractiveCommand`](ocs_plugin_api::host::InteractiveCommand)
/// to the host's internal `CadCommand`, so a plugin tool drives the host's
/// point-collection flow (viewport clicks or `--serve` coordinates) just like a
/// built-in tool.
struct PluginInteractiveAdapter {
    inner: Box<dyn ocs_plugin_api::host::InteractiveCommand>,
}

impl crate::command::CadCommand for PluginInteractiveAdapter {
    fn name(&self) -> &'static str {
        "PLUGIN"
    }
    // Every call into the plugin runs under a panic guard (#145): a buggy plugin
    // that panics mid-command leaves the host running — the command just ends.
    fn prompt(&self) -> String {
        crate::plugin::guard("InteractiveCommand::prompt", || self.inner.prompt())
            .unwrap_or_default()
    }
    fn on_point(&mut self, pt: glam::DVec3) -> crate::command::CmdResult {
        crate::plugin::guard("InteractiveCommand::on_point", || {
            self.inner.on_point([pt.x as f64, pt.y as f64, pt.z as f64])
        })
        .map(plugin_step_to_result)
        .unwrap_or(crate::command::CmdResult::Cancel)
    }
    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::plugin::guard("InteractiveCommand::on_enter", || self.inner.on_enter())
            .map(plugin_step_to_result)
            .unwrap_or(crate::command::CmdResult::Cancel)
    }
    fn needs_entity_pick(&self) -> bool {
        crate::plugin::guard("InteractiveCommand::needs_object_pick", || {
            self.inner.needs_object_pick()
        })
        .unwrap_or(false)
    }
    fn on_entity_pick(&mut self, handle: Handle, pt: glam::DVec3) -> crate::command::CmdResult {
        crate::plugin::guard("InteractiveCommand::on_object_pick", || {
            self.inner
                .on_object_pick(handle, [pt.x as f64, pt.y as f64, pt.z as f64])
        })
        .map(plugin_step_to_result)
        .unwrap_or(crate::command::CmdResult::Cancel)
    }
}

/// Bridges an out-of-process plugin's interactive command to the host's
/// `CadCommand`. Events are sent over IPC and the returned `CommandStep` is
/// translated into a `CmdResult`. Prompt and object-pick mode are cached and
/// refreshed after each event.
pub(crate) struct PluginProcessInteractiveAdapter {
    pub process: std::sync::Arc<ocs_plugin_api::process::PluginProcess>,
    pub command_id: u64,
    prompt: Option<String>,
    needs_entity_pick: Option<bool>,
}

impl PluginProcessInteractiveAdapter {
    pub(crate) fn new(
        process: std::sync::Arc<ocs_plugin_api::process::PluginProcess>,
        command_id: u64,
    ) -> Self {
        let prompt = process.get_prompt(command_id).ok();
        let needs_entity_pick = process.needs_entity_pick(command_id).ok();
        Self {
            process,
            command_id,
            prompt,
            needs_entity_pick,
        }
    }

    fn refresh(&mut self) {
        self.prompt = self.process.get_prompt(self.command_id).ok();
        self.needs_entity_pick = self.process.needs_entity_pick(self.command_id).ok();
    }
}

impl Drop for PluginProcessInteractiveAdapter {
    fn drop(&mut self) {
        let _ = self.process.drop_interactive(self.command_id);
    }
}

impl crate::command::CadCommand for PluginProcessInteractiveAdapter {
    fn name(&self) -> &'static str {
        "PLUGIN"
    }
    fn prompt(&self) -> String {
        self.prompt.clone().unwrap_or_default()
    }
    fn on_point(&mut self, pt: glam::DVec3) -> crate::command::CmdResult {
        use ocs_plugin_api::ipc::protocol::InteractiveEvent;
        let result = self
            .process
            .interactive_event(
                self.command_id,
                InteractiveEvent::Point([pt.x, pt.y, pt.z]),
            )
            .map(plugin_step_to_result)
            .unwrap_or(crate::command::CmdResult::Cancel);
        self.refresh();
        result
    }
    fn on_enter(&mut self) -> crate::command::CmdResult {
        use ocs_plugin_api::ipc::protocol::InteractiveEvent;
        let result = self
            .process
            .interactive_event(self.command_id, InteractiveEvent::Enter)
            .map(plugin_step_to_result)
            .unwrap_or(crate::command::CmdResult::Cancel);
        self.refresh();
        result
    }
    fn needs_entity_pick(&self) -> bool {
        self.needs_entity_pick.unwrap_or(false)
    }
    fn on_entity_pick(&mut self, handle: Handle, pt: glam::DVec3) -> crate::command::CmdResult {
        use ocs_plugin_api::ipc::protocol::InteractiveEvent;
        let result = self
            .process
            .interactive_event(
                self.command_id,
                InteractiveEvent::ObjectPick {
                    handle,
                    pt: [pt.x, pt.y, pt.z],
                },
            )
            .map(plugin_step_to_result)
            .unwrap_or(crate::command::CmdResult::Cancel);
        self.refresh();
        result
    }
}

fn plugin_step_to_result(step: ocs_plugin_api::host::CommandStep) -> crate::command::CmdResult {
    use crate::command::CmdResult;
    use ocs_plugin_api::host::CommandStep;
    match step {
        CommandStep::NeedPoint => CmdResult::NeedPoint,
        CommandStep::Commit(e) => CmdResult::CommitEntity(e),
        CommandStep::CommitAndEnd(e) => CmdResult::CommitAndExit(e),
        CommandStep::Done | CommandStep::Cancel => CmdResult::Cancel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::OpenCADStudio;
    use crate::entities::traits::RenderConvertible;
    use acadrust::entities::{Line, Point};
    use acadrust::xdata::XDataValue;
    use ocs_plugin_api::host::DocumentReader;

    #[test]
    fn selection_query_is_live_and_replacement_is_validated() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let a = host.add_entity(EntityType::Point(Point::new()));
        let b = host.add_entity(EntityType::Point(Point::new()));
        assert!(host.selection().is_empty());
        host.set_selection(&[b, a]).unwrap();
        assert_eq!(host.selection(), vec![b, a]);
        assert!(host.set_selection(&[Handle::new(99999)]).is_err());
        assert!(host.set_selection(&[a, a]).is_err());
        assert_eq!(host.selection(), vec![b, a]);
        host.set_selection(&[]).unwrap();
        assert!(host.selection().is_empty());
    }

    #[test]
    fn scripted_selection_does_not_expand_linked_leader_annotation() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let annotation = host.add_entity(EntityType::Text(acadrust::entities::Text::default()));
        let mut leader = acadrust::entities::Leader::default();
        leader.annotation_handle = annotation;
        let leader_handle = host.add_entity(EntityType::Leader(leader));
        host.set_selection(&[leader_handle]).unwrap();
        assert_eq!(host.selection(), vec![leader_handle]);
    }

    #[test]
    fn entity_transaction_validates_before_mutation_and_undoes_as_one_step() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let (first, second);
        {
            let mut host = HostSession::new(&mut app, 0);
            first = host.add_entity(EntityType::Point(Point::at(acadrust::types::Vector3::new(1.0, 0.0, 0.0))));
            second = host.add_entity(EntityType::Point(Point::at(acadrust::types::Vector3::new(2.0, 0.0, 0.0))));
            let mut a = host.document().get_entity(first).unwrap().clone();
            let mut b = host.document().get_entity(second).unwrap().clone();
            if let EntityType::Point(p) = &mut a { p.location.x = 10.0; }
            if let EntityType::Point(p) = &mut b { p.location.x = 20.0; }
            let mut invalid = b.clone();
            invalid.common_mut().owner_handle = Handle::new(99999);
            assert!(host.update_entities_transaction("Move points", vec![a.clone(), invalid]).is_err());
            let mut invalid_geometry = b.clone();
            if let EntityType::Point(p) = &mut invalid_geometry { p.location.x = f64::NAN; }
            assert!(host.update_entities_transaction("Move points", vec![a.clone(), invalid_geometry])
                .unwrap_err().contains("Point.location"));
            assert_eq!(host.app.tabs[0].history.undo_stack.len(), 0);
            assert!(matches!(host.document().get_entity(first), Some(EntityType::Point(p)) if p.location.x == 1.0));
            host.update_entities_transaction("Move points", vec![a, b]).unwrap();
            assert!(matches!(host.document().get_entity(first), Some(EntityType::Point(p)) if p.location.x == 10.0));
        }
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 1);
        app.undo_steps(1);
        assert!(matches!(app.tabs[0].scene.document.get_entity(first), Some(EntityType::Point(p)) if p.location.x == 1.0));
        assert!(matches!(app.tabs[0].scene.document.get_entity(second), Some(EntityType::Point(p)) if p.location.x == 2.0));
        app.redo_steps(1);
        assert!(matches!(app.tabs[0].scene.document.get_entity(first), Some(EntityType::Point(p)) if p.location.x == 10.0));
        assert!(matches!(app.tabs[0].scene.document.get_entity(second), Some(EntityType::Point(p)) if p.location.x == 20.0));
    }

    /// Opt-in end-to-end check using the staged Python cdylib and the actual
    /// OCS executable as its out-of-process runner. Run with OCS_TEST_PYTHON_PLUGIN
    /// and OCS_PLUGIN_RUNNER_EXE set; unlike the local-socket protocol test,
    /// this executes Python and calls the app's HostSession over real IPC.
    #[test]
    fn staged_python_plugin_line_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else { return; };
        let plugin_path = std::path::PathBuf::from(plugin_path);
        assert!(plugin_path.is_file(), "missing staged Python plugin: {}", plugin_path.display());
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());

        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            &plugin_path, &mut host, crate::plugin::v4_support::notification_handler(),
        ).expect("spawn staged Python plugin");
        assert_eq!(process.id(), "opencad.python");
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process.dispatch(host, command, &mut |_| {}).expect("Python dispatch"));
        };
        dispatch(&mut host, concat!(
            "PY_EVAL ocs.active_document.create_entity('Line',",
            "start={'x':0.0,'y':0.0,'z':0.0},",
            "end={'x':1.0,'y':0.0,'z':0.0}).handle"
        ));
        let handles: Vec<_> = host.document().entities().map(|entity| entity.common().handle).collect();
        assert_eq!(handles.len(), 1, "Python add did not reach the host");
        let handle = handles[0];
        assert!(matches!(host.document().get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 1.0));
        let before_edit = host.document().get_entity(handle).unwrap().clone();
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Move line', [{{'handle':{},'end':{{'x':5.0,'y':0.0,'z':0.0}}}}])",
            handle.value()
        ));
        let mut expected_after_edit = before_edit;
        let EntityType::Line(expected_line) = &mut expected_after_edit else { unreachable!() };
        expected_line.end.x = 5.0;
        assert_eq!(host.document().get_entity(handle), Some(&expected_after_edit),
            "partial Python edit must preserve every unmentioned common and geometry field");
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Reject NaN', [{{'handle':{},'start':{{'x':float('nan'),'y':0.0,'z':0.0}}}}])",
            handle.value()
        ));
        assert_eq!(host.document().get_entity(handle), Some(&expected_after_edit),
            "invalid geometry changed the line");
        assert!(host.app.command_line.history.last().unwrap().text.contains("finite coordinates"));
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Reject duplicate', [{{'handle':{},'end':{{'x':8.0,'y':0.0,'z':0.0}}}},{{'handle':{},'end':{{'x':9.0,'y':0.0,'z':0.0}}}}])",
            handle.value(), handle.value()
        ));
        assert!(matches!(host.document().get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 5.0),
            "rejected batch changed the line");
        let dwg_bytes = acadrust::DwgWriter::write_to_vec(host.document()).expect("write edited DWG");
        let dwg_doc = acadrust::DwgReader::from_stream(std::io::Cursor::new(dwg_bytes))
            .read().expect("reopen edited DWG");
        assert!(matches!(dwg_doc.get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 5.0));
        let dxf_bytes = acadrust::DxfWriter::new(host.document()).write_to_vec().expect("write edited DXF");
        let dxf_doc = acadrust::DxfReader::from_reader(std::io::Cursor::new(dxf_bytes))
            .expect("open edited DXF").read().expect("reopen edited DXF");
        assert!(matches!(dxf_doc.get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 5.0));
        dispatch(&mut host, &format!("PY_EVAL ocs.active_document.delete_entity({})", handle.value()));
        assert!(host.document().get_entity(handle).is_none(), "Python removal did not reach the host");
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3,
            "create, edit, and delete should each form one undo entry");
        app.undo_steps(1);
        assert!(matches!(app.tabs[0].scene.document.get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 5.0));
        app.undo_steps(1);
        assert!(matches!(app.tabs[0].scene.document.get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 1.0));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    #[test]
    fn staged_python_tolerance_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else { return; };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path), &mut host,
            crate::plugin::v4_support::notification_handler(),
        ).expect("spawn staged Python plugin");
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process.dispatch(host, command, &mut |_| {}).expect("Python dispatch"));
        };
        dispatch(&mut host, concat!(
            "PY_EVAL ocs.active_document.create_entity('Tolerance',",
            "insertion_point={'x':1.0,'y':2.0,'z':0.0},",
            "text='POSITION%%v0.1').handle"
        ));
        let handle = host.document().entities().next().expect("Python created a Tolerance")
            .common().handle;
        let Some(EntityType::Tolerance(created)) = host.document().get_entity(handle) else {
            panic!("expected Tolerance");
        };
        assert_eq!(created.insertion_point, acadrust::types::Vector3::new(1.0, 2.0, 0.0));
        assert_eq!(created.dimension_style_name, "Standard");
        let created = created.clone();
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.active_document.entities[{}].text", handle.value()));
        assert!(host.app.command_line.history.last().unwrap().text.contains("POSITION%%v0.1"));

        let script_path = std::env::temp_dir().join(format!(
            "ocs_tolerance_document_model_{}.py", std::process::id()));
        std::fs::write(&script_path, format!(concat!(
            "doc = ocs.active_document\n",
            "tolerance = doc.entities[{}]\n",
            "with doc.transaction('Edit tolerance'):\n",
            "    tolerance.insertion_point = (4.0, 5.0, 0.0)\n",
            "    tolerance.direction = (0.0, 1.0, 0.0)\n",
            "    tolerance.text = 'POSITION%%v0.2'\n",
            "doc.selection = [tolerance]\n"
        ), handle.value())).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script_path.display()));
        let _ = std::fs::remove_file(&script_path);
        let expected = host.document().get_entity(handle).unwrap().clone();
        let EntityType::Tolerance(edited) = &expected else { unreachable!() };
        assert_eq!(edited.insertion_point, acadrust::types::Vector3::new(4.0, 5.0, 0.0));
        assert_eq!(edited.direction, acadrust::types::Vector3::UNIT_Y);
        assert_eq!(edited.text, "POSITION%%v0.2");
        assert_eq!(edited.common, created.common);
        assert_eq!(edited.normal, created.normal);
        assert_eq!(edited.dimension_style_name, created.dimension_style_name);
        assert_eq!(edited.dimension_style_handle, created.dimension_style_handle);
        assert_eq!(edited.text_height, created.text_height);
        assert_eq!(edited.dimension_gap, created.dimension_gap);
        assert_eq!(edited.dwg_unknown_short, created.dwg_unknown_short);
        assert_eq!(host.selection(), vec![handle], "Python selection did not reach the canvas");

        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Reject direction', [{{'handle':{},'direction':{{'x':2.0,'y':0.0,'z':0.0}}}}])",
            handle.value()));
        assert_eq!(host.document().get_entity(handle), Some(&expected));
        assert!(host.app.command_line.history.last().unwrap().text.contains("unit vector"));
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Reject style', [{{'handle':{},'dimension_style_name':'Missing'}}])",
            handle.value()));
        assert_eq!(host.document().get_entity(handle), Some(&expected));
        assert!(host.app.command_line.history.last().unwrap().text.contains("does not exist"));

        let dwg_bytes = acadrust::DwgWriter::write_to_vec(host.document()).unwrap();
        let dwg_doc = acadrust::DwgReader::from_stream(std::io::Cursor::new(dwg_bytes)).read().unwrap();
        assert!(matches!(dwg_doc.get_entity(handle), Some(EntityType::Tolerance(value))
            if value.insertion_point.x == 4.0 && value.text == "POSITION%%v0.2"));
        let dxf_bytes = acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap();
        let dxf_doc = acadrust::DxfReader::from_reader(std::io::Cursor::new(dxf_bytes))
            .unwrap().read().unwrap();
        assert!(matches!(dxf_doc.get_entity(handle), Some(EntityType::Tolerance(value))
            if value.insertion_point.x == 4.0 && value.text == "POSITION%%v0.2"));

        dispatch(&mut host, &format!("PY_EVAL ocs.active_document.delete_entity({})", handle.value()));
        assert!(host.document().get_entity(handle).is_none());
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle), Some(&expected));
        app.undo_steps(1);
        assert!(matches!(app.tabs[0].scene.document.get_entity(handle), Some(EntityType::Tolerance(value))
            if value.insertion_point.x == 1.0 && value.text == "POSITION%%v0.1"));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    #[test]
    fn staged_python_shape_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else { return; };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());

        let shx_path = std::env::temp_dir().join(format!("ocs_host_shape_{}.shx", std::process::id()));
        let mut shx = b"AutoCAD-86 shapes 1.0\r\n\x1A".to_vec();
        for value in [1_u16, 1, 1, 1, 9] { shx.extend_from_slice(&value.to_le_bytes()); }
        shx.extend_from_slice(b"ARROW\0\x10\x14\0");
        std::fs::write(&shx_path, shx).unwrap();

        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let style_handle = host.document_mut().allocate_handle();
        let mut style = acadrust::tables::TextStyle::new("TestShapes");
        style.handle = style_handle;
        style.is_shape_file = true;
        style.font_file = shx_path.to_string_lossy().into_owned();
        host.document_mut().text_styles.add(style).unwrap();

        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path), &mut host,
            crate::plugin::v4_support::notification_handler(),
        ).expect("spawn staged Python plugin");
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process.dispatch(host, command, &mut |_| {}).expect("Python dispatch"));
        };
        dispatch(&mut host, concat!(
            "PY_EVAL ocs.active_document.create_entity('Shape',",
            "insertion_point={'x':1.0,'y':2.0,'z':0.0},size=2.0,",
            "shape_name='ARROW',shape_number=1,style_name='TestShapes').handle"
        ));
        let handle = host.document().entities().next().expect("Python created a Shape")
            .common().handle;
        let Some(EntityType::Shape(created)) = host.document().get_entity(handle) else {
            panic!("expected Shape");
        };
        assert_eq!(created.style_handle, Some(style_handle));
        let created = created.clone();
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.active_document.entities[{}].shape_name", handle.value()));
        assert!(host.app.command_line.history.last().unwrap().text.contains("ARROW"));

        let script_path = std::env::temp_dir().join(format!(
            "ocs_shape_document_model_{}.py", std::process::id()));
        std::fs::write(&script_path, format!(concat!(
            "doc = ocs.active_document\n",
            "shape = doc.entities[{}]\n",
            "with doc.transaction('Edit shape'):\n",
            "    shape.insertion_point = (4.0, 5.0, 0.0)\n",
            "    shape.size = 3.0\n",
            "    shape.rotation = 0.5\n",
            "    shape.relative_x_scale = 1.5\n",
            "doc.selection = [shape]\n"
        ), handle.value())).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script_path.display()));
        let _ = std::fs::remove_file(&script_path);
        let expected = host.document().get_entity(handle).unwrap().clone();
        let EntityType::Shape(edited) = &expected else { unreachable!() };
        assert_eq!(edited.insertion_point, acadrust::types::Vector3::new(4.0, 5.0, 0.0));
        assert_eq!(edited.size, 3.0);
        assert_eq!(edited.rotation, 0.5);
        assert_eq!(edited.relative_x_scale, 1.5);
        assert_eq!(edited.common, created.common);
        assert_eq!(edited.shape_name, created.shape_name);
        assert_eq!(edited.shape_number, created.shape_number);
        assert_eq!(edited.style_name, created.style_name);
        assert_eq!(edited.style_handle, created.style_handle);
        assert_eq!(edited.normal, created.normal);
        assert_eq!(edited.thickness, created.thickness);
        assert_eq!(host.selection(), vec![handle]);

        let assert_real_glyph = |entity: &EntityType, document: &CadDocument| {
            let EntityType::Shape(shape) = entity else { panic!("expected Shape"); };
            let render = shape.to_render(document).expect("shape render");
            let crate::scene::convert::acad_to_render::RenderObject::Lines(points) = render.object else {
                panic!("expected shape linework");
            };
            assert_eq!(points.len(), 3, "expected SHX glyph rather than diamond placeholder");
            assert!(points.iter().flatten().all(|value| value.is_finite()));
        };
        assert_real_glyph(&expected, host.document());

        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Reject size', [{{'handle':{},'size':0.0}}])",
            handle.value()));
        assert_eq!(host.document().get_entity(handle), Some(&expected));
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Reject style', [{{'handle':{},'style_name':'Missing'}}])",
            handle.value()));
        assert_eq!(host.document().get_entity(handle), Some(&expected));
        assert!(host.app.command_line.history.last().unwrap().text.contains("does not exist"));

        let dwg_bytes = acadrust::DwgWriter::write_to_vec(host.document()).unwrap();
        let dwg_doc = crate::io::load_bytes("shape.dwg", dwg_bytes).unwrap();
        let dwg_entity = dwg_doc.get_entity(handle).expect("Shape survives DWG");
        assert!(matches!(dwg_entity, EntityType::Shape(value)
            if value.shape_number == 1 && value.size == 3.0 && value.rotation == 0.5));
        assert_real_glyph(dwg_entity, &dwg_doc);
        let dxf_bytes = acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap();
        let dxf_doc = crate::io::load_bytes("shape.dxf", dxf_bytes).unwrap();
        let dxf_entity = dxf_doc.get_entity(handle).expect("Shape survives DXF");
        assert!(matches!(dxf_entity, EntityType::Shape(value)
            if value.shape_name == "ARROW" && value.size == 3.0 && (value.rotation - 0.5).abs() < 1e-12));
        assert_real_glyph(dxf_entity, &dxf_doc);

        dispatch(&mut host, &format!("PY_EVAL ocs.active_document.delete_entity({})", handle.value()));
        assert!(host.document().get_entity(handle).is_none());
        drop(process);
        drop(host);
        let _ = std::fs::remove_file(&shx_path);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle), Some(&expected));
        app.undo_steps(1);
        assert!(matches!(app.tabs[0].scene.document.get_entity(handle), Some(EntityType::Shape(value))
            if value.insertion_point.x == 1.0 && value.size == 2.0));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    #[test]
    fn staged_python_point_pick_and_cancel_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else { return; };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let process = {
            let mut host = HostSession::new(&mut app, 0);
            std::sync::Arc::new(ocs_plugin_api::process::PluginProcess::spawn(
                std::path::Path::new(&plugin_path), &mut host,
                crate::plugin::v4_support::notification_handler(),
            ).expect("spawn staged Python plugin"))
        };
        let request = |app: &mut OpenCADStudio, prompt: &str, entity: bool| {
            let mut started = None;
            {
                let mut host = HostSession::new(app, 0);
                let method = if entity { "request_entity" } else { "request_point" };
                let cmd = format!("PY_EVAL ocs.active_document.{method}('{prompt}')");
                assert!(process.dispatch(&mut host, &cmd, &mut |id| started = Some(id)).unwrap());
            }
            let output = app.command_line.history.last().unwrap().text.clone();
            let token = output.split_whitespace().find_map(|part| part.parse::<u64>().ok())
                .unwrap_or_else(|| panic!("Python pick token missing from {output:?}"));
            let id = started.expect("Python request started an interactive command");
            app.set_active_command(0, Box::new(PluginProcessInteractiveAdapter::new(
                std::sync::Arc::clone(&process), id,
            )));
            token
        };

        let picked_token = request(&mut app, "Pick a point", false);
        let result = app.tabs[0].active_cmd.as_mut().unwrap().on_point(glam::DVec3::new(3.0, 4.0, 0.0));
        let _ = app.apply_cmd_result(result);
        assert!(app.tabs[0].active_cmd.is_none());
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process.dispatch(&mut host, &format!("PY_EVAL ocs.active_document.poll_input({picked_token})"), &mut |_| {}).unwrap());
        }
        assert!(app.command_line.history.last().unwrap().text.contains("point"));

        let target = {
            let mut host = HostSession::new(&mut app, 0);
            host.add_entity(EntityType::Line(Line::from_points(
                acadrust::types::Vector3::new(6.0, 7.0, 0.0),
                acadrust::types::Vector3::new(9.0, 7.0, 0.0),
            )))
        };
        let entity_token = request(&mut app, "Pick an entity", true);
        assert!(app.tabs[0].active_cmd.as_ref().unwrap().needs_entity_pick());
        let result = app.tabs[0].active_cmd.as_mut().unwrap()
            .on_entity_pick(target, glam::DVec3::new(6.0, 7.0, 0.0));
        let _ = app.apply_cmd_result(result);
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process.dispatch(&mut host, &format!("PY_EVAL ocs.active_document.poll_input({entity_token})"), &mut |_| {}).unwrap());
        }
        assert!(app.command_line.history.last().unwrap().text.contains("entity"));
        assert!(app.command_line.history.last().unwrap().text.contains(&target.value().to_string()));

        let cancelled_token = request(&mut app, "Cancel a point", false);
        let result = app.tabs[0].active_cmd.as_mut().unwrap().on_enter();
        let _ = app.apply_cmd_result(result);
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process.dispatch(&mut host, &format!("PY_EVAL ocs.active_document.poll_input({cancelled_token})"), &mut |_| {}).unwrap());
        }
        assert!(app.command_line.history.last().unwrap().text.contains("cancelled"));
    }

    #[test]
    fn staged_python_tabs_isolate_tokens_and_notifications() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else { return; };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        app.tabs.push(crate::app::document::DocumentTab::new_drawing(2));
        app.tabs[1].is_start = false;
        let tab0 = app.tabs[0].id;
        let tab1 = app.tabs[1].id;
        assert_ne!(tab0, tab1);
        let process = {
            let mut host = HostSession::new(&mut app, 0);
            std::sync::Arc::new(ocs_plugin_api::process::PluginProcess::spawn(
                std::path::Path::new(&plugin_path), &mut host,
                crate::plugin::v4_support::notification_handler(),
            ).expect("spawn staged Python plugin"))
        };
        let mut started = None;
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process.dispatch(&mut host, "PY_EVAL ocs.active_document.request_point('Tab zero')",
                &mut |id| started = Some(id)).unwrap());
        }
        let token = app.command_line.history.last().unwrap().text.split_whitespace()
            .find_map(|part| part.parse::<u64>().ok()).expect("pick token");
        app.set_active_command(0, Box::new(PluginProcessInteractiveAdapter::new(
            std::sync::Arc::clone(&process), started.unwrap(),
        )));
        {
            let mut host = HostSession::new(&mut app, 1);
            assert!(process.dispatch(&mut host, &format!("PY_EVAL ocs.active_document.poll_input({token})"), &mut |_| {}).unwrap());
        }
        assert!(app.command_line.history.last().unwrap().text.contains("None"),
            "other tab must not consume the pending token");
        let result = app.tabs[0].active_cmd.as_mut().unwrap().on_point(glam::DVec3::new(2.0, 3.0, 0.0));
        let _ = app.apply_cmd_result(result);
        {
            let mut host = HostSession::new(&mut app, 1);
            assert!(process.dispatch(&mut host, &format!("PY_EVAL ocs.active_document.poll_input({token})"), &mut |_| {}).unwrap());
        }
        assert!(app.command_line.history.last().unwrap().text.contains("None"));
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process.dispatch(&mut host, &format!("PY_EVAL ocs.active_document.poll_input({token})"), &mut |_| {}).unwrap());
        }
        assert!(app.command_line.history.last().unwrap().text.contains("point"));

        use ocs_plugin_api::host::HostNotification;
        process.notify_plugin(None, HostNotification::DrawingChanged { tab_id: tab0, epoch: 101 }).unwrap();
        process.notify_plugin(None, HostNotification::DrawingChanged { tab_id: tab1, epoch: 202 }).unwrap();
        // Notifications are best-effort and consumed by the runner's reader
        // thread before its next Dispatch. Give that thread a bounded handoff.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let poll_events_until = |app: &mut OpenCADStudio, tab: usize, epoch: &str| {
            let mut output = String::new();
            for _ in 0..10 {
                {
                    let mut host = HostSession::new(app, tab);
                    assert!(process.dispatch(&mut host,
                        "PY_EVAL ocs.active_document.poll_events()", &mut |_| {}).unwrap());
                }
                output = app.command_line.history.last().unwrap().text.clone();
                if output.contains(epoch) { break; }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            output
        };
        let other_events = poll_events_until(&mut app, 1, "202");
        assert!(other_events.contains("202") && !other_events.contains("101"), "{other_events}");
        let first_events = poll_events_until(&mut app, 0, "101");
        assert!(first_events.contains("101") && !first_events.contains("202"), "{first_events}");

        for epoch in 0..257 {
            process.notify_plugin(None, HostNotification::DrawingChanged { tab_id: tab0, epoch }).unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process.dispatch(&mut host, "PY_EVAL ocs.active_document.poll_events()", &mut |_| {}).unwrap());
        }
        let overflow = &app.command_line.history.last().unwrap().text;
        assert!(overflow.contains("overflow") && overflow.contains("dropped"), "{overflow}");

        process.notify_plugin(None, HostNotification::DrawingChanged { tab_id: tab0, epoch: 303 }).unwrap();
        process.notify_plugin(None, HostNotification::DocumentTabClosed { tab_id: tab0 }).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process.dispatch(&mut host, "PY_EVAL ocs.active_document.poll_events()", &mut |_| {}).unwrap());
        }
        assert!(app.command_line.history.last().unwrap().text.contains("[]"),
            "closing a tab must discard its queued events");
    }

    #[test]
    fn unmapped_canvas_kind_layer_change_undoes() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let handle;
        {
            let mut host = HostSession::new(&mut app, 0);
            handle = host.add_entity(EntityType::Hatch(acadrust::entities::Hatch::default()));
            let original = host.document().get_entity(handle).unwrap();
            let changed = ocs_plugin_api::entity_coverage::patch_canvas_layer(original, "HATCHES").unwrap();
            host.update_entities_transaction("Move hatch layer", vec![changed]).unwrap();
            assert_eq!(host.document().get_entity(handle).unwrap().common().layer, "HATCHES");
        }
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 1);
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle).unwrap().common().layer, "0");
    }

    #[test]
    fn insert_transform_transaction_undoes_without_changing_block_identity() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let handle;
        {
            let mut host = HostSession::new(&mut app, 0);
            let insert = acadrust::entities::Insert::new("DOOR", acadrust::types::Vector3::new(1.0, 2.0, 0.0));
            handle = host.add_entity(EntityType::Insert(insert));
            let mut changed = host.document().get_entity(handle).unwrap().clone();
            if let EntityType::Insert(insert) = &mut changed {
                insert.insert_point.x = 4.0;
                insert.set_x_scale(2.0);
            }
            host.update_entities_transaction("Move block", vec![changed]).unwrap();
            assert!(matches!(host.document().get_entity(handle), Some(EntityType::Insert(insert))
                if insert.insert_point.x == 4.0 && insert.x_scale() == 2.0 && insert.block_name == "DOOR"));
        }
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 1);
        app.undo_steps(1);
        assert!(matches!(app.tabs[0].scene.document.get_entity(handle), Some(EntityType::Insert(insert))
            if insert.insert_point.x == 1.0 && insert.x_scale() == 1.0 && insert.block_name == "DOOR"));
    }

    #[test]
    fn system_variables_change_without_command_reentry() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        assert_eq!(host.system_variable("clayer"), Some(HostSettingValue::Text("0".into())));
        assert_eq!(host.system_variable("SNAPANG"), Some(HostSettingValue::Number(0.0)));
        assert!(host.set_system_variable("CLAYER", HostSettingValue::Text("Missing".into())).is_err());
        assert_eq!(host.system_variable("CLAYER"), Some(HostSettingValue::Text("0".into())));

        let mut point = Point::new();
        point.common.layer = "Annotations".into();
        host.add_entity(EntityType::Point(point));
        assert_eq!(host.set_system_variable("clayer", HostSettingValue::Text("Annotations".into())),
                   Ok(HostSettingValue::Text("Annotations".into())));
        assert_eq!(host.system_variable("CLAYER"), Some(HostSettingValue::Text("Annotations".into())));
        assert_eq!(host.app.tabs[0].active_layer, "Annotations");
        assert_eq!(host.app.tabs[0].layers.current_layer, "Annotations");
        assert_eq!(host.app.ribbon.active_layer, "Annotations");
        assert!(host.app.tabs[0].dirty);

        assert_eq!(host.set_system_variable("snapang", HostSettingValue::Number(450.0)),
                   Ok(HostSettingValue::Number(90.0)));
        assert_eq!(host.system_variable("SNAPANG"), Some(HostSettingValue::Number(90.0)));
        assert!(host.set_system_variable("SNAPANG", HostSettingValue::Number(f64::INFINITY)).is_err());
        assert!(host.set_system_variable("SNAPANG", HostSettingValue::Number(f64::MAX)).is_err());
        assert_eq!(host.system_variable("SNAPANG"), Some(HostSettingValue::Number(90.0)));
    }

    #[test]
    fn xdata_record_round_trips_and_registers_appid() {
        let mut app = OpenCADStudio::new_for_test();
        let mut host = HostSession::new(&mut app, 0);
        let h = host.add_entity(EntityType::Point(Point::new()));

        let mut rec = ExtendedDataRecord::new("DEMO_SURVEY");
        rec.add_value(XDataValue::String("PNT-1".to_string()));
        rec.add_value(XDataValue::Integer32(42));
        assert!(host.write_record(h, rec));

        let got = host.read_record(h, "DEMO_SURVEY").expect("record missing");
        assert_eq!(got.values.len(), 2);
        // APPID registered so the XDATA survives a DWG/DXF round-trip.
        assert!(host.document().app_ids.contains("DEMO_SURVEY"));

        // A second write replaces rather than duplicates the record.
        let mut rec2 = ExtendedDataRecord::new("DEMO_SURVEY");
        rec2.add_value(XDataValue::String("PNT-2".to_string()));
        assert!(host.write_record(h, rec2));
        let got = host.read_record(h, "DEMO_SURVEY").unwrap();
        assert_eq!(got.values.len(), 1);

        // Removal reports whether anything was dropped.
        assert!(host.remove_record(h, "DEMO_SURVEY"));
        assert!(host.read_record(h, "DEMO_SURVEY").is_none());
        assert!(!host.remove_record(h, "DEMO_SURVEY"));
    }

    #[test]
    fn plugin_state_round_trips_through_hostapi_trait() {
        use ocs_plugin_api::host::{self, HostApi};
        let mut app = OpenCADStudio::new_for_test();
        let mut session = HostSession::new(&mut app, 0);
        let host: &mut dyn HostApi = &mut session;

        // Absent before first use.
        assert!(host::plugin_state::<u32>(&*host, "opencad.demo").is_none());
        // Insert via ensure, then mutate.
        *host::ensure_plugin_state(host, "opencad.demo", || 7u32) += 1;
        assert_eq!(
            *host::plugin_state::<u32>(&*host, "opencad.demo").unwrap(),
            8
        );
        *host::plugin_state_mut::<u32>(host, "opencad.demo").unwrap() = 100;
        assert_eq!(
            *host::plugin_state::<u32>(&*host, "opencad.demo").unwrap(),
            100
        );
    }

    #[test]
    fn add_entities_batch_assigns_handles_and_publishes_once() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);

        let pts: Vec<EntityType> = (0..5)
            .map(|i| {
                EntityType::Point(Point::at(acadrust::types::Vector3::new(
                    i as f64, i as f64, 0.0,
                )))
            })
            .collect();
        let handles = host.add_entities(pts);

        assert_eq!(handles.len(), 5);
        assert!(handles.iter().all(|h| !h.is_null()));
        // Each handle is unique.
        let mut set = std::collections::HashSet::new();
        for h in &handles {
            assert!(set.insert(h.value()));
        }
        // All entities are in the document.
        for h in &handles {
            assert!(host.document().get_entity(*h).is_some());
        }
    }

    #[test]
    fn update_entity_replaces_in_place_preserving_handle() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let h = host.add_entity(EntityType::Point(Point::at(acadrust::types::Vector3::new(
            1.0, 1.0, 0.0,
        ))));
        let epoch_before = host.app.tabs[0].scene.geometry_epoch;

        // Edit a snapshot copy (as a plugin would) and commit it.
        let mut edited = host.document().get_entity(h).unwrap().clone();
        edited.common_mut().layer = "PLUGIN_EDIT".to_string();
        assert!(host.update_entity(edited));

        // Same handle, edit applied, geometry re-tessellated.
        let got = host.document().get_entity(h).expect("entity kept its handle");
        assert_eq!(got.common().layer, "PLUGIN_EDIT");
        assert_ne!(
            host.app.tabs[0].scene.geometry_epoch, epoch_before,
            "update should bump geometry"
        );

        // Updating an unknown handle fails and changes nothing.
        let mut ghost = Point::new();
        ghost.common.handle = Handle::new(999_999);
        assert!(!host.update_entity(EntityType::Point(ghost)));
    }

    #[test]
    fn add_and_update_auto_register_novel_layers_with_real_handles() {
        // A plugin adds/edits an entity naming a layer no LAYER command ever
        // created. The layer must gain a real table entry (non-null handle) so
        // it survives a DWG save instead of collapsing to layer 0 (#252, #67).
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);

        assert!(!host.document().layers.contains("PLUGIN-LAYER"));
        let mut pt = Point::at(acadrust::types::Vector3::new(3.0, 3.0, 0.0));
        pt.common.layer = "PLUGIN-LAYER".to_string();
        host.add_entity(EntityType::Point(pt));

        let layer = host
            .document()
            .layers
            .get("PLUGIN-LAYER")
            .expect("novel layer auto-registered on add_entity");
        assert!(
            !layer.handle.is_null(),
            "auto-registered layer must carry a real handle (#67)"
        );

        // Adding again on the same (now-existing) layer must not duplicate or
        // error — the always-present default "0" is likewise never re-created.
        let count_before = host.document().layers.len();
        host.add_entity(EntityType::Point(Point::new())); // default layer "0"
        let mut pt2 = Point::new();
        pt2.common.layer = "PLUGIN-LAYER".to_string();
        host.add_entity(EntityType::Point(pt2));
        assert_eq!(host.document().layers.len(), count_before);

        // Retargeting an entity to a novel layer via update_entity registers it.
        let h = host.add_entity(EntityType::Point(Point::new()));
        let mut edited = host.document().get_entity(h).unwrap().clone();
        edited.common_mut().layer = "EDIT-LAYER".to_string();
        assert!(host.update_entity(edited));
        assert!(host.document().layers.contains("EDIT-LAYER"));
    }

    #[test]
    fn remove_entity_deletes_and_clears_caches() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let h = host.add_entity(EntityType::Point(Point::at(acadrust::types::Vector3::new(
            2.0, 2.0, 0.0,
        ))));
        assert!(host.document().get_entity(h).is_some());

        assert!(host.remove_entity(h));
        assert!(host.document().get_entity(h).is_none());
        assert!(!host.app.tabs[0].scene.hatches.contains_key(&h));
        assert!(!host.app.tabs[0].scene.meshes.contains_key(&h));

        // Removing an already-gone handle reports false.
        assert!(!host.remove_entity(h));
    }

    /// A plugin command: second point commits a Point and ends.
    struct PlacePoint {
        got_first: bool,
    }
    impl ocs_plugin_api::host::InteractiveCommand for PlacePoint {
        fn prompt(&self) -> String {
            crate::t!("Pick a point").into_owned()
        }
        fn on_point(&mut self, pt: [f64; 3]) -> ocs_plugin_api::host::CommandStep {
            use ocs_plugin_api::host::CommandStep;
            if self.got_first {
                let p = acadrust::entities::Point::at(acadrust::types::Vector3::new(
                    pt[0], pt[1], pt[2],
                ));
                CommandStep::CommitAndEnd(acadrust::EntityType::Point(p))
            } else {
                self.got_first = true;
                CommandStep::NeedPoint
            }
        }
    }

    #[test]
    fn plugin_interactive_command_drives_host_flow() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        {
            let mut host = HostSession::new(&mut app, 0);
            host.start_interactive(Box::new(PlacePoint { got_first: false }));
        }
        assert!(app.tabs[0].active_cmd.is_some());
        for pt in [glam::DVec3::new(0.0, 0.0, 0.0), glam::DVec3::new(5.0, 5.0, 0.0)] {
            let r = app.tabs[0].active_cmd.as_mut().unwrap().on_point(pt);
            let _ = app.apply_cmd_result(r);
        }
        assert_eq!(app.tabs[0].scene.document.entities().count(), 1);
        assert!(app.tabs[0].active_cmd.is_none(), "command should have ended");
    }

    /// A plugin command that picks an existing object, then marks it.
    struct PickThenMark;
    impl ocs_plugin_api::host::InteractiveCommand for PickThenMark {
        fn prompt(&self) -> String {
            crate::t!("Pick an object").into_owned()
        }
        fn on_point(&mut self, _pt: [f64; 3]) -> ocs_plugin_api::host::CommandStep {
            ocs_plugin_api::host::CommandStep::Cancel
        }
        fn needs_object_pick(&self) -> bool {
            true
        }
        fn on_object_pick(
            &mut self,
            _handle: acadrust::Handle,
            pt: [f64; 3],
        ) -> ocs_plugin_api::host::CommandStep {
            let p =
                acadrust::entities::Point::at(acadrust::types::Vector3::new(pt[0], pt[1], pt[2]));
            ocs_plugin_api::host::CommandStep::CommitAndEnd(acadrust::EntityType::Point(p))
        }
    }

    #[test]
    fn plugin_object_pick_routes_to_command() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let target = {
            let mut host = HostSession::new(&mut app, 0);
            let h = host.add_entity(acadrust::EntityType::Point(acadrust::entities::Point::at(
                acadrust::types::Vector3::new(3.0, 4.0, 0.0),
            )));
            host.start_interactive(Box::new(PickThenMark));
            h
        };
        // The command requested an entity pick, not a free point.
        assert!(app.tabs[0].active_cmd.as_ref().unwrap().needs_entity_pick());
        let r = app.tabs[0]
            .active_cmd
            .as_mut()
            .unwrap()
            .on_entity_pick(target, glam::DVec3::new(3.0, 4.0, 0.0));
        let _ = app.apply_cmd_result(r);
        // Original point + the mark the command committed.
        assert_eq!(app.tabs[0].scene.document.entities().count(), 2);
    }

    #[test]
    fn host_document_reader_sees_entities() {
        use ocs_plugin_api::host::ReaderEntityKind;
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        host.add_entity(acadrust::EntityType::Point(acadrust::entities::Point::at(
            acadrust::types::Vector3::new(7.0, 8.0, 0.0),
        )));
        let reader = host.document_reader();
        assert_eq!(reader.entity_count(), 1);
        let mut kinds = Vec::new();
        reader.for_each_entity(&mut |e| kinds.push(e.kind));
        assert_eq!(kinds, vec![ReaderEntityKind::Point]);
    }

    #[test]
    fn host_document_view_publish_and_read_shared() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let info = host.document_view().unwrap();
        let reader =
            ocs_plugin_api::shm::SharedDocumentReader::<ocs_plugin_api::shm::DocumentViewData>::open(std::path::Path::new(&info.path))
                .unwrap();
        assert_eq!(reader.entity_count(), 0);

        host.add_entity(acadrust::EntityType::Point(acadrust::entities::Point::at(
            acadrust::types::Vector3::new(1.0, 2.0, 0.0),
        )));

        assert_eq!(reader.entity_count(), 1);
    }

    /// Read an entity handle from the live document, write XDATA for that
    /// handle, read it back, and remove it.
    #[test]
    fn document_reader_to_xdata_roundtrip() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let h = host.add_entity(EntityType::Point(Point::at(acadrust::types::Vector3::new(
            7.0, 8.0, 0.0,
        ))));

        {
            let reader = host.document_reader();
            assert_eq!(reader.entity_count(), 1);
            let mut handles = Vec::new();
            reader.for_each_entity(&mut |e| handles.push(e.handle));
            assert_eq!(handles, vec![h]);
        }

        let mut rec = ExtendedDataRecord::new("ROUNDTRIP");
        rec.add_value(XDataValue::String("from-reader".to_string()));
        assert!(host.write_record(h, rec));

        let got = host.read_record(h, "ROUNDTRIP").expect("record missing");
        assert_eq!(got.values.len(), 1);
        assert!(matches!(got.values[0], XDataValue::String(ref s) if s == "from-reader"));

        assert!(host.remove_record(h, "ROUNDTRIP"));
        assert!(host.read_record(h, "ROUNDTRIP").is_none());
    }

    /// Publish a shared document view, read the entity handle from shared
    /// memory, then write and read-back XDATA through the normal HostApi RPCs.
    #[test]
    fn shared_document_view_read_then_write_xdata_roundtrip() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let info = host.document_view().unwrap();
        let reader =
            ocs_plugin_api::shm::SharedDocumentReader::<ocs_plugin_api::shm::DocumentViewData>::open(std::path::Path::new(&info.path))
                .unwrap();

        let h = host.add_entity(EntityType::Point(Point::at(acadrust::types::Vector3::new(
            1.0, 2.0, 0.0,
        ))));
        assert_eq!(reader.entity_count(), 1);

        let mut handles = Vec::new();
        reader.for_each_entity(&mut |e| handles.push(e.handle));
        assert_eq!(handles, vec![h]);

        let mut rec = ExtendedDataRecord::new("SHM_ROUNDTRIP");
        rec.add_value(XDataValue::Integer32(123));
        assert!(host.write_record(h, rec));

        let got = host
            .read_record(h, "SHM_ROUNDTRIP")
            .expect("record missing");
        assert_eq!(got.values.len(), 1);
        assert!(matches!(got.values[0], XDataValue::Integer32(123)));
    }
}
