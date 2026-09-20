// HostSession — plugin-facing API implemented inside `app` (private field access).

use std::any::Any;

use acadrust::tables::AppId;
use acadrust::xdata::ExtendedDataRecord;
use ocs_plugin_api::host::{CadDocument, EntityType, Handle, HostApi, HostSettingValue};
use ocs_plugin_api::shm::{DocumentSnapshotStore, DocumentViewData};

use super::OpenCADStudio;
#[cfg(not(target_arch = "wasm32"))]
use crate::plugin::v4_support;

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
    pub fn document_view_v4(
        &mut self,
        tab_id: u64,
    ) -> Option<ocs_plugin_api::shm::DocumentViewInfo> {
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

    fn nested_attribute(&self, handle: Handle) -> Option<EntityType> {
        self.document().entities().find_map(|entity| {
            let EntityType::Insert(insert) = entity else {
                return None;
            };
            insert
                .attributes
                .iter()
                .find(|attribute| attribute.common.handle == handle)
                .cloned()
                .map(EntityType::AttributeEntity)
        })
    }

    fn host_model_entity(&self, handle: Handle) -> Option<EntityType> {
        self.document()
            .get_entity(handle)
            .cloned()
            .or_else(|| self.nested_attribute(handle))
    }

    fn replace_nested_attribute(&mut self, attribute: acadrust::entities::AttributeEntity) -> bool {
        let owner = attribute.common.owner_handle;
        let handle = attribute.common.handle;
        let Some(EntityType::Insert(mut insert)) = self.document().get_entity(owner).cloned()
        else {
            return false;
        };
        let Some(slot) = insert
            .attributes
            .iter_mut()
            .find(|candidate| candidate.common.handle == handle)
        else {
            return false;
        };
        *slot = attribute;
        self.app.tabs[self.tab]
            .scene
            .update_entity(EntityType::Insert(insert))
    }

    pub fn document_view(&mut self) -> Option<ocs_plugin_api::shm::DocumentViewInfo> {
        if self.doc_store.is_none() {
            let mut store =
                DocumentSnapshotStore::<DocumentViewData>::new(self.tab as u64, 8 * 1024 * 1024)
                    .ok()?;
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

    /// Recompute host-derived state for entities a script created or edited.
    fn normalize_scripted_entity(
        &self,
        old: Option<&EntityType>,
        entity: &mut EntityType,
    ) -> Result<(), String> {
        if let EntityType::Dimension(dimension) = entity {
            crate::entities::dimension::normalize_scripted_dimension(dimension);
        }
        if let EntityType::Helix(new) = entity {
            let old = match old {
                Some(EntityType::Helix(old)) => Some(old),
                _ => None,
            };
            crate::entities::helix::normalize_scripted_helix(old, new)?;
        }
        if let EntityType::Table(new) = entity {
            let old = match old {
                Some(EntityType::Table(old)) => Some(old),
                _ => None,
            };
            crate::entities::table::normalize_scripted_table(old, new);
        }
        if let EntityType::MLine(new) = entity {
            let old = match old {
                Some(EntityType::MLine(old)) => Some(old),
                _ => None,
            };
            crate::entities::mline::normalize_scripted_mline(old, new, self.document())?;
        }
        Ok(())
    }

    /// A scripted RasterImage names a file; the host creates the image
    /// definition object it needs (pixel size read from the file) and links it.
    fn prepare_scripted_raster_image(&mut self, entity: &mut EntityType) -> Result<(), String> {
        let EntityType::RasterImage(image) = entity else {
            return Ok(());
        };
        if image.definition_handle.is_some_and(|handle| !handle.is_null()) {
            return Ok(());
        }
        let (width, height) = image::image_dimensions(&image.file_path)
            .map_err(|error| format!("cannot read image {:?}: {error}", image.file_path))?;
        let handle = self.document_mut().allocate_handle();
        let mut definition = acadrust::objects::ImageDefinition::with_dimensions(
            image.file_path.clone(),
            width,
            height,
        );
        definition.handle = handle;
        definition.is_loaded = true;
        self.document_mut()
            .objects
            .insert(handle, acadrust::objects::ObjectType::ImageDefinition(definition));
        image.definition_handle = Some(handle);
        // `size` is the image's pixel size, and an untouched clip boundary
        // covers the whole image, so both follow the file.
        let untouched_clip = image.clip_boundary
            == acadrust::entities::ClipBoundary::full_image(image.size.x, image.size.y);
        image.size = acadrust::types::Vector2::new(f64::from(width), f64::from(height));
        if untouched_clip {
            image.clip_boundary = acadrust::entities::ClipBoundary::full_image(
                f64::from(width),
                f64::from(height),
            );
        }
        Ok(())
    }

    pub fn add_entity(&mut self, mut entity: EntityType) -> Handle {
        if self.prepare_scripted_raster_image(&mut entity).is_err() {
            return Handle::NULL;
        }
        if self.normalize_scripted_entity(None, &mut entity).is_err() {
            return Handle::NULL;
        }
        if let EntityType::AttributeEntity(mut attribute) = entity {
            let owner = attribute.common.owner_handle;
            let Some(EntityType::Insert(mut insert)) = self.document().get_entity(owner).cloned()
            else {
                return Handle::NULL;
            };
            if attribute.common.handle.is_null() {
                attribute.common.handle = self.document_mut().allocate_handle();
            }
            let handle = attribute.common.handle;
            if insert
                .attributes
                .iter()
                .any(|existing| existing.common.handle == handle)
            {
                return Handle::NULL;
            }
            insert.attributes.push(attribute);
            if !self.app.tabs[self.tab]
                .scene
                .update_entity(EntityType::Insert(insert))
            {
                return Handle::NULL;
            }
            self.publish_document_view();
            return handle;
        }
        let handle = self.app.tabs[self.tab].scene.add_entity(entity);
        self.publish_document_view();
        handle
    }

    pub fn add_entities(&mut self, mut entities: Vec<EntityType>) -> Vec<Handle> {
        for entity in &mut entities {
            let _ = self.prepare_scripted_raster_image(entity);
            // Creation input was validated by the caller; a degenerate MLine
            // simply keeps its supplied (unnormalized) vertices.
            let _ = self.normalize_scripted_entity(None, entity);
        }
        let handles = self.app.tabs[self.tab].scene.add_entities(entities);
        self.publish_document_view();
        handles
    }

    pub fn bump_geometry(&mut self) {
        self.app.tabs[self.tab].scene.bump_geometry();
    }

    /// Replace the entity carrying `entity`'s handle in place, refreshing the
    /// scene's derived caches. Returns `false` when no entity has that handle.
    pub fn update_entity(&mut self, mut entity: EntityType) -> bool {
        let old = self.document().get_entity(entity.common().handle).cloned();
        if self.normalize_scripted_entity(old.as_ref(), &mut entity).is_err() {
            return false;
        }
        let ok = match entity {
            EntityType::AttributeEntity(attribute) => self.replace_nested_attribute(attribute),
            entity => self.app.tabs[self.tab].scene.update_entity(entity),
        };
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
        if label.trim().is_empty() {
            return Err("transaction label is empty".into());
        }
        if entities.is_empty() {
            return Ok(());
        }
        let mut seen = HashSet::new();
        for entity in &mut entities {
            let handle = entity.common().handle;
            if handle.is_null() || !seen.insert(handle) {
                return Err(format!("null or duplicate entity handle: {handle:?}"));
            }
            let Some(existing) = self.host_model_entity(handle) else {
                return Err(format!("entity {handle:?} does not exist"));
            };
            if std::mem::discriminant(&existing) != std::mem::discriminant(entity) {
                return Err(format!("entity {handle:?} changes kind"));
            }
            if existing.common().owner_handle != entity.common().owner_handle {
                return Err(format!("entity {handle:?} changes owner"));
            }
            ocs_plugin_api::entity_coverage::bind_canvas_entity_references(self.document(), entity)
                .map_err(|error| format!("entity {handle:?}: {error}"))?;
            ocs_plugin_api::entity_coverage::validate_entity_mutation(&existing, entity)
                .map_err(|error| format!("entity {handle:?}: {error}"))?;
            self.normalize_scripted_entity(Some(&existing), entity)
                .map_err(|error| format!("entity {handle:?}: {error}"))?;
            let lock_handle = if matches!(entity, EntityType::AttributeEntity(_)) {
                entity.common().owner_handle
            } else {
                handle
            };
            if self.app.tabs[self.tab].scene.is_layer_locked(lock_handle) {
                return Err(format!("entity {handle:?} is on a locked layer"));
            }
        }
        self.push_undo(label);
        for entity in entities {
            // Validation above makes this infallible while the session owns
            // the document exclusively.
            let updated = match entity {
                EntityType::AttributeEntity(attribute) => self.replace_nested_attribute(attribute),
                entity => self.app.tabs[self.tab].scene.update_entity(entity),
            };
            assert!(updated);
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
        if self.selection() == handles {
            return Ok(());
        }
        let scene = &mut self.app.tabs[self.tab].scene;
        scene.replace_selection_exact(handles);
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.tab == self.app.active_tab {
                self.app.last_plugin_selection = Some((
                    self.tab_id(),
                    self.app.tabs[self.tab].scene.selection_fingerprint(),
                ));
            }
            crate::plugin::v4_support::publish_selection_changed_v4(
                self.tab_id(),
                self.selection(),
            );
        }
        Ok(())
    }

    /// Delete the entity with `handle`, keeping the scene's render caches in
    /// sync. Returns `false` when the entity is absent or on a locked layer
    /// (which `erase_entities` refuses to remove).
    pub fn remove_entity(&mut self, handle: Handle) -> bool {
        if self.document().get_entity(handle).is_none() {
            let Some(EntityType::AttributeEntity(attribute)) = self.nested_attribute(handle) else {
            return false;
            };
            let owner = attribute.common.owner_handle;
            let Some(EntityType::Insert(mut insert)) = self.document().get_entity(owner).cloned()
            else {
                return false;
            };
            insert
                .attributes
                .retain(|candidate| candidate.common.handle != handle);
            let removed = self.app.tabs[self.tab]
                .scene
                .update_entity(EntityType::Insert(insert));
            if removed {
                self.publish_document_view();
            }
            return removed;
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
        let app_handle = self
            .document()
            .app_ids
            .get(app_name)
            .map(|a| a.handle.value());
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
            ("CLAYER", _) | ("SNAPANG", _) => {
                Err(format!("{}: wrong value type", name.to_ascii_uppercase()))
            }
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
    fn update_entities_transaction(
        &mut self,
        label: &str,
        entities: Vec<EntityType>,
    ) -> Result<(), String> {
        self.update_entities_transaction(label, entities)
    }
    fn selection(&self) -> Vec<Handle> {
        self.selection()
    }
    fn set_selection(&mut self, handles: &[Handle]) -> Result<(), String> {
        self.set_selection(handles)
    }
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
            .interactive_event(self.command_id, InteractiveEvent::Point([pt.x, pt.y, pt.z]))
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
    fn nested_insert_attribute_crud_uses_parent_storage_and_undo() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let (insert_handle, attribute_handle);
        {
            let mut host = HostSession::new(&mut app, 0);
            let next = host.document().next_handle();
            let record_handle = Handle::new(next);
            let block_handle = Handle::new(next + 1);
            let block_end_handle = Handle::new(next + 2);
            let mut record = acadrust::tables::BlockRecord::new("TAGBLOCK");
            record.handle = record_handle;
            record.block_entity_handle = block_handle;
            record.block_end_handle = block_end_handle;
            host.document_mut().block_records.add(record).unwrap();
            let mut block =
                acadrust::entities::Block::new("TAGBLOCK", acadrust::types::Vector3::ZERO);
            block.common.handle = block_handle;
            block.common.owner_handle = record_handle;
            host.document_mut()
                .add_entity(EntityType::Block(block))
                .unwrap();
            let mut end = acadrust::entities::BlockEnd::new();
            end.common.handle = block_end_handle;
            end.common.owner_handle = record_handle;
            host.document_mut()
                .add_entity(EntityType::BlockEnd(end))
                .unwrap();
            let mut definition = acadrust::entities::AttributeDefinition::new(
                "PART_NO".into(),
                "Part number".into(),
                "PN-001".into(),
            );
            definition.common.owner_handle = record_handle;
            let definition_handle = host
                .document_mut()
                .add_entity(EntityType::AttributeDefinition(definition))
                .unwrap();
            insert_handle = host
                .document_mut()
                .add_entity(EntityType::Insert(acadrust::entities::Insert::new(
                    "TAGBLOCK",
                    acadrust::types::Vector3::new(10.0, 0.0, 0.0),
                )))
                .unwrap();

            let mut attribute =
                acadrust::entities::AttributeEntity::new("PART_NO".into(), "PN-101".into());
            attribute.common.owner_handle = insert_handle;
            attribute.attdef_handle = definition_handle;
            attribute.insertion_point = acadrust::types::Vector3::new(10.0, 2.0, 0.0);
            host.push_undo("Create attribute");
            attribute_handle = host.add_entity(EntityType::AttributeEntity(attribute));
            assert!(!attribute_handle.is_null());
            assert!(
                host.document().get_entity(attribute_handle).is_none(),
                "nested attributes must not become orphan flat entities"
            );
            assert!(matches!(host.host_model_entity(attribute_handle),
                Some(EntityType::AttributeEntity(value)) if value.value == "PN-101"));
            let view = ocs_plugin_api::shm::DocumentViewDataV4::from(host.document());
            assert!(view
                .entities
                .iter()
                .any(|entity| entity.handle == attribute_handle.value()));

            let mut changed = host.host_model_entity(attribute_handle).unwrap();
            let EntityType::AttributeEntity(value) = &mut changed else {
                unreachable!()
            };
            value.value = "PN-102".into();
            value.insertion_point.x = 12.0;
            host.update_entities_transaction("Edit attribute", vec![changed])
                .unwrap();
            assert!(matches!(host.host_model_entity(attribute_handle),
                Some(EntityType::AttributeEntity(value))
                    if value.value == "PN-102" && value.insertion_point.x == 12.0));

            host.push_undo("Delete attribute");
            assert!(host.remove_entity(attribute_handle));
            assert!(host.host_model_entity(attribute_handle).is_none());
        }
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(insert_handle),
            Some(EntityType::Insert(insert)) if insert.attributes.len() == 1
                && insert.attributes[0].value == "PN-102")
        );
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(insert_handle),
            Some(EntityType::Insert(insert)) if insert.attributes[0].value == "PN-101")
        );
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(insert_handle),
            Some(EntityType::Insert(insert)) if insert.attributes.is_empty())
        );
        app.redo_steps(3);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(insert_handle),
            Some(EntityType::Insert(insert)) if insert.attributes.is_empty())
        );
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
            first = host.add_entity(EntityType::Point(Point::at(acadrust::types::Vector3::new(
                1.0, 0.0, 0.0,
            ))));
            second = host.add_entity(EntityType::Point(Point::at(acadrust::types::Vector3::new(
                2.0, 0.0, 0.0,
            ))));
            let mut a = host.document().get_entity(first).unwrap().clone();
            let mut b = host.document().get_entity(second).unwrap().clone();
            if let EntityType::Point(p) = &mut a {
                p.location.x = 10.0;
            }
            if let EntityType::Point(p) = &mut b {
                p.location.x = 20.0;
            }
            let mut invalid = b.clone();
            invalid.common_mut().owner_handle = Handle::new(99999);
            assert!(host
                .update_entities_transaction("Move points", vec![a.clone(), invalid])
                .is_err());
            let mut invalid_geometry = b.clone();
            if let EntityType::Point(p) = &mut invalid_geometry {
                p.location.x = f64::NAN;
            }
            assert!(host
                .update_entities_transaction("Move points", vec![a.clone(), invalid_geometry])
                .unwrap_err()
                .contains("Point.location"));
            assert_eq!(host.app.tabs[0].history.undo_stack.len(), 0);
            assert!(
                matches!(host.document().get_entity(first), Some(EntityType::Point(p)) if p.location.x == 1.0)
            );
            host.update_entities_transaction("Move points", vec![a, b])
                .unwrap();
            assert!(
                matches!(host.document().get_entity(first), Some(EntityType::Point(p)) if p.location.x == 10.0)
            );
        }
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 1);
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(first), Some(EntityType::Point(p)) if p.location.x == 1.0)
        );
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(second), Some(EntityType::Point(p)) if p.location.x == 2.0)
        );
        app.redo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(first), Some(EntityType::Point(p)) if p.location.x == 10.0)
        );
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(second), Some(EntityType::Point(p)) if p.location.x == 20.0)
        );
    }

    /// Opt-in end-to-end check using the staged Python cdylib and the actual
    /// OCS executable as its out-of-process runner. Run with OCS_TEST_PYTHON_PLUGIN
    /// and OCS_PLUGIN_RUNNER_EXE set; unlike the local-socket protocol test,
    /// this executes Python and calls the app's HostSession over real IPC.
    #[test]
    fn staged_python_plugin_line_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let plugin_path = std::path::PathBuf::from(plugin_path);
        assert!(
            plugin_path.is_file(),
            "missing staged Python plugin: {}",
            plugin_path.display()
        );
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());

        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            &plugin_path,
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .expect("spawn staged Python plugin");
        assert_eq!(process.id(), "opencad.python");
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process
                .dispatch(host, command, &mut |_| {})
                .expect("Python dispatch"));
        };
        dispatch(
            &mut host,
            concat!(
            "PY_EVAL ocs.active_document.create_entity('Line',",
            "start={'x':0.0,'y':0.0,'z':0.0},",
            "end={'x':1.0,'y':0.0,'z':0.0}).handle"
            ),
        );
        let handles: Vec<_> = host
            .document()
            .entities()
            .map(|entity| entity.common().handle)
            .collect();
        assert_eq!(handles.len(), 1, "Python add did not reach the host");
        let handle = handles[0];
        assert!(
            matches!(host.document().get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 1.0)
        );
        let before_edit = host.document().get_entity(handle).unwrap().clone();
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Move line', [{{'handle':{},'end':{{'x':5.0,'y':0.0,'z':0.0}}}}])",
            handle.value()
        ));
        let mut expected_after_edit = before_edit;
        let EntityType::Line(expected_line) = &mut expected_after_edit else {
            unreachable!()
        };
        expected_line.end.x = 5.0;
        assert_eq!(
            host.document().get_entity(handle),
            Some(&expected_after_edit),
            "partial Python edit must preserve every unmentioned common and geometry field"
        );
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Reject NaN', [{{'handle':{},'start':{{'x':float('nan'),'y':0.0,'z':0.0}}}}])",
            handle.value()
        ));
        assert_eq!(
            host.document().get_entity(handle),
            Some(&expected_after_edit),
            "invalid geometry changed the line"
        );
        assert!(host
            .app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("finite coordinates"));
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Reject duplicate', [{{'handle':{},'end':{{'x':8.0,'y':0.0,'z':0.0}}}},{{'handle':{},'end':{{'x':9.0,'y':0.0,'z':0.0}}}}])",
            handle.value(), handle.value()
        ));
        assert!(
            matches!(host.document().get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 5.0),
            "rejected batch changed the line"
        );
        let dwg_bytes =
            acadrust::DwgWriter::write_to_vec(host.document()).expect("write edited DWG");
        let dwg_doc = acadrust::DwgReader::from_stream(std::io::Cursor::new(dwg_bytes))
            .read()
            .expect("reopen edited DWG");
        assert!(
            matches!(dwg_doc.get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 5.0)
        );
        let dxf_bytes = acadrust::DxfWriter::new(host.document())
            .write_to_vec()
            .expect("write edited DXF");
        let dxf_doc = acadrust::DxfReader::from_reader(std::io::Cursor::new(dxf_bytes))
            .expect("open edited DXF")
            .read()
            .expect("reopen edited DXF");
        assert!(
            matches!(dxf_doc.get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 5.0)
        );
        dispatch(
            &mut host,
            &format!(
                "PY_EVAL ocs.active_document.delete_entity({})",
                handle.value()
            ),
        );
        assert!(
            host.document().get_entity(handle).is_none(),
            "Python removal did not reach the host"
        );
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(
            app.tabs[0].history.undo_stack.len(),
            3,
            "create, edit, and delete should each form one undo entry"
        );
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 5.0)
        );
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(handle), Some(EntityType::Line(line)) if line.end.x == 1.0)
        );
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    #[test]
    fn staged_python_tolerance_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .expect("spawn staged Python plugin");
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process
                .dispatch(host, command, &mut |_| {})
                .expect("Python dispatch"));
        };
        dispatch(
            &mut host,
            concat!(
            "PY_EVAL ocs.active_document.create_entity('Tolerance',",
            "insertion_point={'x':1.0,'y':2.0,'z':0.0},",
            "text='POSITION%%v0.1').handle"
            ),
        );
        let handle = host
            .document()
            .entities()
            .next()
            .expect("Python created a Tolerance")
            .common()
            .handle;
        let Some(EntityType::Tolerance(created)) = host.document().get_entity(handle) else {
            panic!("expected Tolerance");
        };
        assert_eq!(
            created.insertion_point,
            acadrust::types::Vector3::new(1.0, 2.0, 0.0)
        );
        assert_eq!(created.dimension_style_name, "Standard");
        let created = created.clone();
        dispatch(
            &mut host,
            &format!(
                "PY_EVAL ocs.active_document.entities[{}].text",
                handle.value()
            ),
        );
        assert!(host
            .app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("POSITION%%v0.1"));

        let script_path = std::env::temp_dir().join(format!(
            "ocs_tolerance_document_model_{}.py",
            std::process::id()
        ));
        std::fs::write(
            &script_path,
            format!(
                concat!(
            "doc = ocs.active_document\n",
            "tolerance = doc.entities[{}]\n",
            "with doc.transaction('Edit tolerance'):\n",
            "    tolerance.insertion_point = (4.0, 5.0, 0.0)\n",
            "    tolerance.direction = (0.0, 1.0, 0.0)\n",
            "    tolerance.text = 'POSITION%%v0.2'\n",
            "doc.selection = [tolerance]\n"
                ),
                handle.value()
            ),
        )
        .unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script_path.display()));
        let _ = std::fs::remove_file(&script_path);
        let expected = host.document().get_entity(handle).unwrap().clone();
        let EntityType::Tolerance(edited) = &expected else {
            unreachable!()
        };
        assert_eq!(
            edited.insertion_point,
            acadrust::types::Vector3::new(4.0, 5.0, 0.0)
        );
        assert_eq!(edited.direction, acadrust::types::Vector3::UNIT_Y);
        assert_eq!(edited.text, "POSITION%%v0.2");
        assert_eq!(edited.common, created.common);
        assert_eq!(edited.normal, created.normal);
        assert_eq!(edited.dimension_style_name, created.dimension_style_name);
        assert_eq!(
            edited.dimension_style_handle,
            created.dimension_style_handle
        );
        assert_eq!(edited.text_height, created.text_height);
        assert_eq!(edited.dimension_gap, created.dimension_gap);
        assert_eq!(edited.dwg_unknown_short, created.dwg_unknown_short);
        assert_eq!(
            host.selection(),
            vec![handle],
            "Python selection did not reach the canvas"
        );

        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Reject direction', [{{'handle':{},'direction':{{'x':2.0,'y':0.0,'z':0.0}}}}])",
            handle.value()));
        assert_eq!(host.document().get_entity(handle), Some(&expected));
        assert!(host
            .app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("unit vector"));
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Reject style', [{{'handle':{},'dimension_style_name':'Missing'}}])",
            handle.value()));
        assert_eq!(host.document().get_entity(handle), Some(&expected));
        assert!(host
            .app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("does not exist"));

        let dwg_bytes = acadrust::DwgWriter::write_to_vec(host.document()).unwrap();
        let dwg_doc = acadrust::DwgReader::from_stream(std::io::Cursor::new(dwg_bytes))
            .read()
            .unwrap();
        assert!(
            matches!(dwg_doc.get_entity(handle), Some(EntityType::Tolerance(value))
            if value.insertion_point.x == 4.0 && value.text == "POSITION%%v0.2")
        );
        let dxf_bytes = acadrust::DxfWriter::new(host.document())
            .write_to_vec()
            .unwrap();
        let dxf_doc = acadrust::DxfReader::from_reader(std::io::Cursor::new(dxf_bytes))
            .unwrap()
            .read()
            .unwrap();
        assert!(
            matches!(dxf_doc.get_entity(handle), Some(EntityType::Tolerance(value))
            if value.insertion_point.x == 4.0 && value.text == "POSITION%%v0.2")
        );

        dispatch(
            &mut host,
            &format!(
                "PY_EVAL ocs.active_document.delete_entity({})",
                handle.value()
            ),
        );
        assert!(host.document().get_entity(handle).is_none());
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(
            app.tabs[0].scene.document.get_entity(handle),
            Some(&expected)
        );
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(handle), Some(EntityType::Tolerance(value))
            if value.insertion_point.x == 1.0 && value.text == "POSITION%%v0.1")
        );
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    #[test]
    fn staged_python_shape_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());

        let shx_path =
            std::env::temp_dir().join(format!("ocs_host_shape_{}.shx", std::process::id()));
        let mut shx = b"AutoCAD-86 shapes 1.0\r\n\x1A".to_vec();
        for value in [1_u16, 1, 1, 1, 9] {
            shx.extend_from_slice(&value.to_le_bytes());
        }
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
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .expect("spawn staged Python plugin");
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process
                .dispatch(host, command, &mut |_| {})
                .expect("Python dispatch"));
        };
        dispatch(
            &mut host,
            concat!(
            "PY_EVAL ocs.active_document.create_entity('Shape',",
            "insertion_point={'x':1.0,'y':2.0,'z':0.0},size=2.0,",
            "shape_name='ARROW',shape_number=1,style_name='TestShapes').handle"
            ),
        );
        let handle = host
            .document()
            .entities()
            .next()
            .expect("Python created a Shape")
            .common()
            .handle;
        let Some(EntityType::Shape(created)) = host.document().get_entity(handle) else {
            panic!("expected Shape");
        };
        assert_eq!(created.style_handle, Some(style_handle));
        let created = created.clone();
        dispatch(
            &mut host,
            &format!(
                "PY_EVAL ocs.active_document.entities[{}].shape_name",
                handle.value()
            ),
        );
        assert!(host
            .app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("ARROW"));

        let script_path = std::env::temp_dir().join(format!(
            "ocs_shape_document_model_{}.py",
            std::process::id()
        ));
        std::fs::write(
            &script_path,
            format!(
                concat!(
            "doc = ocs.active_document\n",
            "shape = doc.entities[{}]\n",
            "with doc.transaction('Edit shape'):\n",
            "    shape.insertion_point = (4.0, 5.0, 0.0)\n",
            "    shape.size = 3.0\n",
            "    shape.rotation = 0.5\n",
            "    shape.relative_x_scale = 1.5\n",
            "doc.selection = [shape]\n"
                ),
                handle.value()
            ),
        )
        .unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script_path.display()));
        let _ = std::fs::remove_file(&script_path);
        let expected = host.document().get_entity(handle).unwrap().clone();
        let EntityType::Shape(edited) = &expected else {
            unreachable!()
        };
        assert_eq!(
            edited.insertion_point,
            acadrust::types::Vector3::new(4.0, 5.0, 0.0)
        );
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
            let EntityType::Shape(shape) = entity else {
                panic!("expected Shape");
            };
            let render = shape.to_render(document).expect("shape render");
            let crate::scene::convert::acad_to_render::RenderObject::Lines(points) = render.object
            else {
                panic!("expected shape linework");
            };
            assert_eq!(
                points.len(),
                3,
                "expected SHX glyph rather than diamond placeholder"
            );
            assert!(points.iter().flatten().all(|value| value.is_finite()));
        };
        assert_real_glyph(&expected, host.document());

        dispatch(
            &mut host,
            &format!(
            "PY_EVAL ocs.update_many('Reject size', [{{'handle':{},'size':0.0}}])",
                handle.value()
            ),
        );
        assert_eq!(host.document().get_entity(handle), Some(&expected));
        dispatch(
            &mut host,
            &format!(
            "PY_EVAL ocs.update_many('Reject style', [{{'handle':{},'style_name':'Missing'}}])",
                handle.value()
            ),
        );
        assert_eq!(host.document().get_entity(handle), Some(&expected));
        assert!(host
            .app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("does not exist"));

        let dwg_bytes = acadrust::DwgWriter::write_to_vec(host.document()).unwrap();
        let dwg_doc = crate::io::load_bytes("shape.dwg", dwg_bytes).unwrap();
        let dwg_entity = dwg_doc.get_entity(handle).expect("Shape survives DWG");
        assert!(matches!(dwg_entity, EntityType::Shape(value)
            if value.shape_number == 1 && value.size == 3.0 && value.rotation == 0.5));
        assert_real_glyph(dwg_entity, &dwg_doc);
        let dxf_bytes = acadrust::DxfWriter::new(host.document())
            .write_to_vec()
            .unwrap();
        let dxf_doc = crate::io::load_bytes("shape.dxf", dxf_bytes).unwrap();
        let dxf_entity = dxf_doc.get_entity(handle).expect("Shape survives DXF");
        assert!(matches!(dxf_entity, EntityType::Shape(value)
            if value.shape_name == "ARROW" && value.size == 3.0 && (value.rotation - 0.5).abs() < 1e-12));
        assert_real_glyph(dxf_entity, &dxf_doc);

        dispatch(
            &mut host,
            &format!(
                "PY_EVAL ocs.active_document.delete_entity({})",
                handle.value()
            ),
        );
        assert!(host.document().get_entity(handle).is_none());
        drop(process);
        drop(host);
        let _ = std::fs::remove_file(&shx_path);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(
            app.tabs[0].scene.document.get_entity(handle),
            Some(&expected)
        );
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(handle), Some(EntityType::Shape(value))
            if value.insertion_point.x == 1.0 && value.size == 2.0)
        );
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    #[test]
    fn staged_python_attribute_definition_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());

        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let next = host.document().next_handle();
        let block_record_handle = Handle::new(next);
        let block_handle = Handle::new(next + 1);
        let block_end_handle = Handle::new(next + 2);
        let mut block_record = acadrust::tables::BlockRecord::new("TAGBLOCK");
        block_record.handle = block_record_handle;
        block_record.block_entity_handle = block_handle;
        block_record.block_end_handle = block_end_handle;
        host.document_mut().block_records.add(block_record).unwrap();
        let mut block = acadrust::entities::Block::new("TAGBLOCK", acadrust::types::Vector3::ZERO);
        block.common.handle = block_handle;
        block.common.owner_handle = block_record_handle;
        host.document_mut()
            .add_entity(EntityType::Block(block))
            .unwrap();
        let mut block_end = acadrust::entities::BlockEnd::new();
        block_end.common.handle = block_end_handle;
        block_end.common.owner_handle = block_record_handle;
        host.document_mut()
            .add_entity(EntityType::BlockEnd(block_end))
            .unwrap();
        let insert = acadrust::entities::Insert::new(
            "TAGBLOCK",
            acadrust::types::Vector3::new(10.0, 0.0, 0.0),
        );
        let insert_handle = host
            .document_mut()
            .add_entity(EntityType::Insert(insert))
            .unwrap();

        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .expect("spawn staged Python plugin");
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process
                .dispatch(host, command, &mut |_| {})
                .expect("Python dispatch"));
        };
        dispatch(
            &mut host,
            &format!(
                concat!(
            "PY_EVAL ocs.active_document.create_entity('AttributeDefinition',",
            "owner_handle={},tag='PART_NO',prompt='Part number',default_value='PN-001',",
            "insertion_point={{'x':1.0,'y':2.0,'z':0.0}},height=2.5).handle"
                ),
                block_record_handle.value()
            ),
        );
        let handle = host
            .document()
            .entities()
            .find(|entity| matches!(entity, EntityType::AttributeDefinition(_)))
            .expect("Python created an AttributeDefinition")
            .common()
            .handle;
        let Some(EntityType::AttributeDefinition(created)) = host.document().get_entity(handle)
        else {
            panic!("expected AttributeDefinition");
        };
        assert_eq!(created.common.owner_handle, block_record_handle);
        assert_eq!(created.tag, "PART_NO");
        assert_eq!(created.text_style, "Standard");
        assert!(host
            .document()
            .block_records
            .get("TAGBLOCK")
            .unwrap()
            .entity_handles
            .contains(&handle));
        let created = created.clone();
        dispatch(&mut host, &format!(
            "PY_EVAL (ocs.active_document.entities[{}].tag, ocs.active_document.entities[{}].owner_handle)",
            handle.value(), handle.value()));
        let readback = &host.app.command_line.history.last().unwrap().text;
        assert!(
            readback.contains("PART_NO")
                && readback.contains(&block_record_handle.value().to_string())
        );

        let script_path = std::env::temp_dir().join(format!(
            "ocs_attribute_definition_document_model_{}.py",
            std::process::id()
        ));
        std::fs::write(
            &script_path,
            format!(
                concat!(
            "doc = ocs.active_document\n",
            "definition = doc.entities[{}]\n",
            "with doc.transaction('Edit attribute definition'):\n",
            "    definition.insertion_point = (4.0, 5.0, 0.0)\n",
            "    definition.default_value = 'PN-002'\n",
            "    definition.rotation = 0.25\n",
            "    definition.width_factor = 1.25\n",
            "doc.selection = [definition]\n"
                ),
                handle.value()
            ),
        )
        .unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script_path.display()));
        let _ = std::fs::remove_file(&script_path);
        let expected = host.document().get_entity(handle).unwrap().clone();
        let EntityType::AttributeDefinition(edited) = &expected else {
            unreachable!()
        };
        assert_eq!(
            edited.insertion_point,
            acadrust::types::Vector3::new(4.0, 5.0, 0.0)
        );
        assert_eq!(edited.default_value, "PN-002");
        assert_eq!(edited.rotation, 0.25);
        assert_eq!(edited.width_factor, 1.25);
        assert_eq!(edited.common, created.common);
        assert_eq!(edited.tag, created.tag);
        assert_eq!(edited.prompt, created.prompt);
        assert_eq!(edited.text_style, created.text_style);
        assert_eq!(edited.flags, created.flags);
        assert_eq!(edited.embedded_mtext, created.embedded_mtext);
        assert_eq!(host.selection(), vec![handle]);
        assert!(
            matches!(host.document().get_entity(insert_handle), Some(EntityType::Insert(value))
            if value.block_name == "TAGBLOCK")
        );

        let render = edited
            .to_render(host.document())
            .expect("attribute definition render");
        let crate::scene::convert::acad_to_render::RenderObject::Text(strokes) = render.object
        else {
            panic!("expected attribute definition text");
        };
        assert!(!strokes.is_empty());
        assert!(strokes.iter().any(|stroke| stroke
            .run
            .as_ref()
            .is_some_and(|run| run.text.contains("PN-002"))));

        for (label, patch, message) in [
            ("Reject tag", "'tag':'BAD TAG'", "whitespace"),
            ("Reject height", "'height':0.0", "greater than zero"),
            ("Reject style", "'text_style':'Missing'", "does not exist"),
            ("Reject owner", "'owner_handle':999999", "read-only"),
        ] {
            dispatch(
                &mut host,
                &format!(
                    "PY_EVAL ocs.update_many({label:?}, [{{'handle':{}, {patch}}}])",
                    handle.value()
                ),
            );
            assert_eq!(host.document().get_entity(handle), Some(&expected));
            assert!(host
                .app
                .command_line
                .history
                .last()
                .unwrap()
                .text
                .contains(message));
        }

        let assert_persisted = |label: &str, document: &CadDocument, full_text_state: bool| {
            let actual = document.get_entity(handle);
            assert!(
                matches!(actual, Some(EntityType::AttributeDefinition(value))
                if value.common.owner_handle == block_record_handle
                    && value.tag == "PART_NO" && value.default_value == "PN-002"
                    && value.insertion_point == acadrust::types::Vector3::new(4.0, 5.0, 0.0)
                    && (value.rotation - 0.25).abs() < 1e-12),
                "{label}: {actual:?}"
            );
            if full_text_state {
                assert!(
                    matches!(actual, Some(EntityType::AttributeDefinition(value))
                    if value.width_factor == 1.25 && value.text_style == "Standard"
                        && value.prompt == "Part number"),
                    "{label}: {actual:?}"
                );
            }
            assert!(document
                .block_records
                .get("TAGBLOCK")
                .unwrap()
                .entity_handles
                .contains(&handle));
            assert!(
                matches!(document.get_entity(insert_handle), Some(EntityType::Insert(value))
                if value.block_name == "TAGBLOCK")
            );
        };
        let dwg_bytes = acadrust::DwgWriter::write_to_vec(host.document()).unwrap();
        let dwg_doc = crate::io::load_bytes("attribute-definition.dwg", dwg_bytes).unwrap();
        assert_persisted("DWG", &dwg_doc, true);
        let dxf_bytes = acadrust::DxfWriter::new(host.document())
            .write_to_vec()
            .unwrap();
        let dxf_doc = crate::io::load_bytes("attribute-definition.dxf", dxf_bytes).unwrap();
        // The pinned cadcodec DXF ATTDEF reader retains identity, placement,
        // height, value, prompt and rotation, but currently drops several
        // optional AcDbText fields such as width factor. Keep that external
        // codec gap explicit in the coverage ledger rather than claiming a
        // complete DXF property round trip here.
        assert_persisted("DXF", &dxf_doc, false);

        dispatch(
            &mut host,
            &format!(
                "PY_EVAL ocs.active_document.delete_entity({})",
                handle.value()
            ),
        );
        assert!(host.document().get_entity(handle).is_none());
        // CadDocument intentionally leaves the block membership slot in place
        // so delta undo can restore the shared entity without rewriting the
        // record. Writers must still omit the absent entity.
        let deleted_dwg = acadrust::DwgWriter::write_to_vec(host.document()).unwrap();
        assert!(
            crate::io::load_bytes("attribute-definition-deleted.dwg", deleted_dwg)
                .unwrap()
                .get_entity(handle)
                .is_none()
        );
        let deleted_dxf = acadrust::DxfWriter::new(host.document())
            .write_to_vec()
            .unwrap();
        assert!(
            crate::io::load_bytes("attribute-definition-deleted.dxf", deleted_dxf)
                .unwrap()
                .get_entity(handle)
                .is_none()
        );
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(
            app.tabs[0].scene.document.get_entity(handle),
            Some(&expected)
        );
        app.undo_steps(1);
        assert!(matches!(app.tabs[0].scene.document.get_entity(handle),
            Some(EntityType::AttributeDefinition(value))
                if value.default_value == "PN-001" && value.insertion_point.x == 1.0));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    #[test]
    fn staged_python_attribute_entity_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let next = host.document().next_handle();
        let record_handle = Handle::new(next);
        let block_handle = Handle::new(next + 1);
        let end_handle = Handle::new(next + 2);
        let mut record = acadrust::tables::BlockRecord::new("TAGBLOCK");
        record.handle = record_handle;
        record.block_entity_handle = block_handle;
        record.block_end_handle = end_handle;
        host.document_mut().block_records.add(record).unwrap();
        let mut block = acadrust::entities::Block::new("TAGBLOCK", acadrust::types::Vector3::ZERO);
        block.common.handle = block_handle;
        block.common.owner_handle = record_handle;
        host.document_mut()
            .add_entity(EntityType::Block(block))
            .unwrap();
        let mut end = acadrust::entities::BlockEnd::new();
        end.common.handle = end_handle;
        end.common.owner_handle = record_handle;
        host.document_mut()
            .add_entity(EntityType::BlockEnd(end))
            .unwrap();
        let mut definition = acadrust::entities::AttributeDefinition::new(
            "PART_NO".into(),
            "Part number".into(),
            "PN-001".into(),
        );
        definition.common.owner_handle = record_handle;
        definition.insertion_point = acadrust::types::Vector3::new(0.0, 2.0, 0.0);
        let definition_handle = host
            .document_mut()
            .add_entity(EntityType::AttributeDefinition(definition))
            .unwrap();
        let insert_handle = host
            .document_mut()
            .add_entity(EntityType::Insert(acadrust::entities::Insert::new(
                "TAGBLOCK",
                acadrust::types::Vector3::new(10.0, 0.0, 0.0),
            )))
            .unwrap();

        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .unwrap();
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process
                .dispatch(host, command, &mut |_| {})
                .expect("Python dispatch"));
        };
        dispatch(
            &mut host,
            &format!(
                concat!(
                    "PY_EVAL ocs.active_document.create_entity('AttributeEntity',owner_handle={},",
                    "tag='PART_NO',value='PN-101',",
                    "insertion_point={{'x':10.0,'y':2.0,'z':0.0}},height=2.5).handle"
                ),
                insert_handle.value()
            ),
        );
        let created_insert = match host.document().get_entity(insert_handle).unwrap() {
            EntityType::Insert(insert) => insert,
            _ => unreachable!(),
        };
        assert_eq!(created_insert.attributes.len(), 1);
        let attribute_handle = created_insert.attributes[0].common.handle;
        assert_eq!(
            created_insert.attributes[0].attdef_handle,
            definition_handle
        );
        let created = created_insert.attributes[0].clone();
        dispatch(
            &mut host,
            &format!(
                "PY_EVAL ocs.active_document.entities[{}].value",
                attribute_handle.value()
            ),
        );
        let readback = &host.app.command_line.history.last().unwrap().text;
        assert!(readback.contains("PN-101"), "{readback}");

        let script =
            std::env::temp_dir().join(format!("ocs_attribute_entity_{}.py", std::process::id()));
        std::fs::write(
            &script,
            format!(
                concat!(
                    "doc = ocs.active_document\n",
                    "attribute = doc.entities[{}]\n",
                    "with doc.transaction('Edit attribute'):\n",
                    "    attribute.value = 'PN-102'\n",
                    "    attribute.insertion_point = (12.0, 3.0, 0.0)\n",
                    "    attribute.rotation = 0.25\n",
                    "doc.selection = [doc.entities[{}]]\n"
                ),
                attribute_handle.value(),
                insert_handle.value()
            ),
        )
        .unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let expected = host.nested_attribute(attribute_handle).unwrap();
        let EntityType::AttributeEntity(edited) = &expected else {
            unreachable!()
        };
        assert_eq!(edited.value, "PN-102");
        assert_eq!(
            edited.insertion_point,
            acadrust::types::Vector3::new(12.0, 3.0, 0.0)
        );
        assert_eq!(edited.common, created.common);
        assert_eq!(edited.attdef_handle, definition_handle);
        assert_eq!(host.selection(), vec![insert_handle]);
        let render = edited.to_render(host.document()).unwrap();
        assert!(matches!(render.object,
            crate::scene::convert::acad_to_render::RenderObject::Text(ref strokes)
                if !strokes.is_empty()));

        for (patch, message) in [
            ("'tag':'MISSING'", "no definition"),
            ("'height':0.0", "greater than zero"),
            ("'owner_handle':999999", "read-only"),
        ] {
            dispatch(
                &mut host,
                &format!(
                    "PY_EVAL ocs.update_many('Reject attribute', [{{'handle':{}, {patch}}}])",
                    attribute_handle.value()
                ),
            );
            assert_eq!(
                host.nested_attribute(attribute_handle),
                Some(expected.clone())
            );
            assert!(host
                .app
                .command_line
                .history
                .last()
                .unwrap()
                .text
                .contains(message));
        }

        let assert_saved = |document: &CadDocument| {
            assert!(
                matches!(document.get_entity(insert_handle), Some(EntityType::Insert(insert))
                if insert.attributes.len() == 1
                    && insert.attributes[0].common.handle == attribute_handle
                    && insert.attributes[0].value == "PN-102"
                    && (insert.attributes[0].rotation - 0.25).abs() < 1e-12)
            );
        };
        assert_saved(
            &crate::io::load_bytes(
                "attribute.dwg",
                acadrust::DwgWriter::write_to_vec(host.document()).unwrap(),
            )
            .unwrap(),
        );
        assert_saved(
            &crate::io::load_bytes(
                "attribute.dxf",
                acadrust::DxfWriter::new(host.document())
                    .write_to_vec()
                    .unwrap(),
            )
            .unwrap(),
        );

        dispatch(
            &mut host,
            &format!(
                "PY_EVAL ocs.active_document.delete_entity({})",
                attribute_handle.value()
            ),
        );
        assert!(host.nested_attribute(attribute_handle).is_none());
        let assert_deleted = |document: &CadDocument| {
            assert!(
                matches!(document.get_entity(insert_handle), Some(EntityType::Insert(insert))
                    if insert.attributes.is_empty())
            );
        };
        assert_deleted(
            &crate::io::load_bytes(
                "attribute-deleted.dwg",
                acadrust::DwgWriter::write_to_vec(host.document()).unwrap(),
            )
            .unwrap(),
        );
        assert_deleted(
            &crate::io::load_bytes(
                "attribute-deleted.dxf",
                acadrust::DxfWriter::new(host.document())
                    .write_to_vec()
                    .unwrap(),
            )
            .unwrap(),
        );
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(insert_handle),
            Some(EntityType::Insert(insert)) if insert.attributes.len() == 1
                && insert.attributes[0].value == "PN-102")
        );
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(insert_handle),
            Some(EntityType::Insert(insert)) if insert.attributes[0].value == "PN-101")
        );
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(insert_handle),
            Some(EntityType::Insert(insert)) if insert.attributes.is_empty())
        );
        app.redo_steps(3);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(insert_handle),
            Some(EntityType::Insert(insert)) if insert.attributes.is_empty())
        );
    }

    #[test]
    fn staged_python_hatch_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .unwrap();
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process.dispatch(host, command, &mut |_| {}).expect("Python dispatch"));
        };
        let script = std::env::temp_dir().join(format!("ocs_hatch_create_{}.py", std::process::id()));
        std::fs::write(&script, concat!(
            "def p(x, y): return {'x': x, 'y': y}\n",
            "def boundary(size):\n",
            "    pairs = [(0.0,0.0,size,0.0),(size,0.0,size,size),(size,size,0.0,size),(0.0,size,0.0,0.0)]\n",
            "    edges = [{'kind':'Line','value':{'start':p(a,b),'end':p(c,d)}} for a,b,c,d in pairs]\n",
            "    return {'flags':1,'edges':edges,'boundary_handles':[]}\n",
            "doc = ocs.active_document\n",
            "hatch = doc.create_entity('Hatch', paths=[boundary(10.0)])\n",
        )).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let hatch_handle = host.document().entities().find_map(|entity| match entity {
            EntityType::Hatch(hatch) => Some(hatch.common.handle),
            _ => None,
        }).unwrap_or_else(|| panic!("Python did not create Hatch: {:?}",
            host.app.command_line.history.last().map(|entry| &entry.text)));
        let created = host.document().get_entity(hatch_handle).unwrap().clone();
        assert!(host.app.tabs[0].scene.hatches.get(&hatch_handle)
            .is_some_and(|model| model.boundary.len() >= 4 && model.name == "SOLID"));
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.active_document.entities[{}].pattern['name']", hatch_handle.value()));
        assert!(host.app.command_line.history.last().unwrap().text.contains("SOLID"));

        let script = std::env::temp_dir().join(format!("ocs_hatch_edit_{}.py", std::process::id()));
        std::fs::write(&script, format!(concat!(
            "def p(x, y): return {{'x': x, 'y': y}}\n",
            "def boundary(size):\n",
            "    pairs = [(0.0,0.0,size,0.0),(size,0.0,size,size),(size,size,0.0,size),(0.0,size,0.0,0.0)]\n",
            "    edges = [{{'kind':'Line','value':{{'start':p(a,b),'end':p(c,d)}}}} for a,b,c,d in pairs]\n",
            "    return {{'flags':1,'edges':edges,'boundary_handles':[]}}\n",
            "doc = ocs.active_document\n",
            "hatch = doc.entities[{}]\n",
            "with doc.transaction('Edit hatch'):\n",
            "    hatch.paths = [boundary(12.0)]\n",
            "    hatch.is_solid = False\n",
            "    hatch.pattern = {{'name':'TEST','description':'test pattern','lines':[",
            "{{'angle':0.0,'base_point':p(0.0,0.0),'offset':p(0.0,2.0),'dash_lengths':[1.0,-1.0]}}]}}\n",
            "    hatch.pattern_scale = 2.0\n",
            "    hatch.pattern_angle = 0.25\n",
            "doc.selection = [hatch]\n"
        ), hatch_handle.value())).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let expected = host.document().get_entity(hatch_handle).unwrap().clone();
        let EntityType::Hatch(edited) = &expected else { unreachable!() };
        assert!(!edited.is_solid);
        assert_eq!(edited.pattern.name, "TEST");
        assert_eq!(edited.pattern.lines.len(), 1);
        assert_eq!(edited.paths[0].edges.len(), 4);
        assert!((edited.pattern_scale - 2.0).abs() < 1e-12);
        assert_eq!(edited.gradient_color, match &created { EntityType::Hatch(value) => value.gradient_color.clone(), _ => unreachable!() });
        assert_eq!(host.selection(), vec![hatch_handle]);
        assert!(host.app.tabs[0].scene.hatches.get(&hatch_handle).is_some_and(|model|
            model.boundary.len() >= 4 && model.name == "TEST"
                && matches!(&model.pattern,
                    crate::scene::model::hatch_model::HatchPattern::Pattern(lines)
                        if !lines.is_empty())));

        for (patch, message) in [
            ("'pattern_scale':0.0", "greater than zero"),
            ("'is_associative':True", "requires boundary handles"),
            ("'gradient_color':{}", "outside the editable schema"),
            ("'paths':[{'flags':1,'edges':[{'kind':'Line','value':{'start':{'x':0.0,'y':0.0},'end':{'x':1.0,'y':0.0}}}],'boundary_handles':[]}]", "not closed"),
        ] {
            dispatch(&mut host, &format!(
                "PY_EVAL ocs.update_many('Reject hatch', [{{'handle':{}, {patch}}}])",
                hatch_handle.value()));
            assert_eq!(host.document().get_entity(hatch_handle), Some(&expected));
            assert!(host.app.command_line.history.last().unwrap().text.contains(message));
        }

        let assert_saved = |document: &CadDocument| {
            assert!(matches!(document.get_entity(hatch_handle), Some(EntityType::Hatch(hatch))
                if !hatch.is_solid && hatch.pattern.name == "TEST"
                    && hatch.pattern.lines.len() == 1 && hatch.paths[0].edges.len() == 4
                    && (hatch.pattern_scale - 2.0).abs() < 1e-12));
        };
        let reopened_dwg = crate::io::load_bytes("hatch.dwg",
            acadrust::DwgWriter::write_to_vec(host.document()).unwrap()).unwrap();
        let reopened_dxf = crate::io::load_bytes("hatch.dxf",
            acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()).unwrap();
        assert_saved(&reopened_dwg);
        assert_saved(&reopened_dxf);
        for (name, mut document) in [("hatch-reedit.dwg", reopened_dwg),
            ("hatch-reedit.dxf", reopened_dxf)] {
            let before = document.get_entity(hatch_handle).unwrap().clone();
            let mut after = before.clone();
            let EntityType::Hatch(hatch) = &mut after else { unreachable!() };
            hatch.elevation = 3.0;
            ocs_plugin_api::entity_coverage::validate_entity_mutation(&before, &after).unwrap();
            *document.get_entity_mut(hatch_handle).unwrap() = after;
            let bytes = if name.ends_with("dwg") {
                acadrust::DwgWriter::write_to_vec(&document).unwrap()
            } else {
                acadrust::DxfWriter::new(&document).write_to_vec().unwrap()
            };
            assert!(matches!(crate::io::load_bytes(name, bytes).unwrap().get_entity(hatch_handle),
                Some(EntityType::Hatch(hatch)) if (hatch.elevation - 3.0).abs() < 1e-12));
        }
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.active_document.delete_entity({})", hatch_handle.value()));
        assert!(host.document().get_entity(hatch_handle).is_none());
        for (name, bytes) in [
            ("hatch-deleted.dwg", acadrust::DwgWriter::write_to_vec(host.document()).unwrap()),
            ("hatch-deleted.dxf", acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()),
        ] {
            assert!(crate::io::load_bytes(name, bytes).unwrap().get_entity(hatch_handle).is_none());
        }
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(hatch_handle), Some(&expected));
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(hatch_handle), Some(&created));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(hatch_handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(hatch_handle).is_none());
    }

    #[test]
    fn staged_python_leader_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .unwrap();
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process.dispatch(host, command, &mut |_| {}).expect("Python dispatch"));
        };
        let last_output = |host: &HostSession<'_>| host.app.command_line.history.last().unwrap().text.clone();

        dispatch(&mut host, concat!(
            "PY_EVAL ocs.active_document.create_entity('Text', text='NOTE', ",
            "insertion={'x':20.0,'y':10.0,'z':0.0}, height=2.5).handle"
        ));
        let text_handle = host.document().entities().find_map(|entity| match entity {
            EntityType::Text(text) => Some(text.common.handle),
            _ => None,
        }).unwrap_or_else(|| panic!("Python did not create Text: {}", last_output(&host)));
        let script = std::env::temp_dir().join(format!("ocs_leader_create_{}.py", std::process::id()));
        std::fs::write(&script, format!(concat!(
            "doc = ocs.active_document\n",
            "doc.create_entity('Leader', vertices=[{{'x':0.0,'y':0.0,'z':0.0}},",
            "{{'x':10.0,'y':10.0,'z':0.0}},{{'x':20.0,'y':10.0,'z':0.0}}], ",
            "annotation_handle={}, hookline_enabled=True)\n"
        ), text_handle.value())).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let leader_handle = host.document().entities().find_map(|entity| match entity {
            EntityType::Leader(leader) => Some(leader.common.handle),
            _ => None,
        }).unwrap_or_else(|| panic!("Python did not create Leader: {}", last_output(&host)));
        let created = host.document().get_entity(leader_handle).unwrap().clone();
        let EntityType::Leader(created_leader) = &created else { unreachable!() };
        assert_eq!(created_leader.annotation_handle, text_handle);
        assert_eq!(created_leader.vertices.len(), 3);

        let assert_geometry = |entity: &EntityType, document: &CadDocument| {
            let EntityType::Leader(leader) = entity else { panic!("expected Leader") };
            let render = leader.to_render(document).expect("leader render");
            let crate::scene::convert::acad_to_render::RenderObject::Lines(points) = render.object else {
                panic!("expected leader linework");
            };
            assert!(points.len() >= leader.vertices.len());
            assert!(points.iter().flatten().all(|value| value.is_nan() || value.is_finite()));
        };
        assert_geometry(&created, host.document());
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.active_document.entities[{}].annotation_handle", leader_handle.value()));
        assert!(last_output(&host).contains(&text_handle.value().to_string()));

        // GUI picking groups a leader with its annotation, while scripted
        // selection stays exact.
        host.app.tabs[0].scene.select_entities(&[leader_handle]);
        let mut grouped = host.selection();
        grouped.sort_by_key(|handle| handle.value());
        assert_eq!(grouped, vec![text_handle, leader_handle]);
        host.app.tabs[0].scene.replace_selection_exact(&[]);

        let script = std::env::temp_dir().join(format!("ocs_leader_edit_{}.py", std::process::id()));
        std::fs::write(&script, format!(concat!(
            "doc = ocs.active_document\n",
            "leader = doc.entities[{}]\n",
            "with doc.transaction('Edit leader'):\n",
            "    leader.vertices = [{{'x':1.0,'y':1.0,'z':0.0}},{{'x':11.0,'y':12.0,'z':0.0}},",
            "{{'x':22.0,'y':12.0,'z':0.0}},{{'x':30.0,'y':12.0,'z':0.0}}]\n",
            "    leader.text_height = 4.0\n",
            "    leader.arrow_enabled = False\n",
            "    leader.annotation_offset = (1.0, 2.0, 0.0)\n",
            "    leader.override_color = {{'kind':'Rgb','value':{{'r':255,'g':0,'b':0}}}}\n",
            "doc.selection = [leader]\n"
        ), leader_handle.value())).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let expected = host.document().get_entity(leader_handle).unwrap().clone();
        let EntityType::Leader(edited) = &expected else { unreachable!() };
        assert_eq!(edited.vertices.len(), 4, "edit failed: {}", last_output(&host));
        assert_eq!(edited.text_height, 4.0);
        assert!(!edited.arrow_enabled);
        assert_eq!(edited.annotation_offset, acadrust::types::Vector3::new(1.0, 2.0, 0.0));
        assert_eq!(edited.annotation_handle, text_handle);
        assert_eq!(edited.common, created_leader.common);
        assert_eq!(edited.dimension_style, created_leader.dimension_style);
        assert_eq!(edited.hookline_enabled, created_leader.hookline_enabled);
        assert_eq!(edited.normal, created_leader.normal);
        assert_eq!(edited.origin, created_leader.origin);
        assert_eq!(edited.override_color, acadrust::types::Color::Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(host.selection(), vec![leader_handle]);
        assert_geometry(&expected, host.document());

        for (patch, message) in [
            ("'annotation_handle':999999", "does not exist"),
            ("'creation_type':'WithTolerance'", "does not match creation_type"),
            ("'creation_type':'NoAnnotation'", "no annotation"),
            ("'vertices':[{'x':0.0,'y':0.0,'z':0.0}]", "at least 2"),
            ("'text_height':0.0", "greater than zero"),
            ("'dimension_style':'Missing'", "dimension style"),
            ("'override_color':{'kind':'Index','value':300}", "Color"),
        ] {
            dispatch(&mut host, &format!(
                "PY_EVAL ocs.update_many('Reject leader', [{{'handle':{}, {patch}}}])",
                leader_handle.value()));
            assert_eq!(host.document().get_entity(leader_handle), Some(&expected), "{patch}");
            assert!(last_output(&host).contains(message), "{patch}: {}", last_output(&host));
        }
        // A valid edit paired with an invalid one is atomic across entities.
        dispatch(&mut host, &format!(concat!(
            "PY_EVAL ocs.update_many('Atomic', [{{'handle':{},'text_width':5.0}},",
            "{{'handle':{},'text_height':0.0}}])"
        ), leader_handle.value(), text_handle.value()));
        assert_eq!(host.document().get_entity(leader_handle), Some(&expected));

        // Both formats keep the path, annotation link, arrow flag and offset.
        // DWG R2010+ derives text_height/text_width/hookline_enabled instead of
        // storing them (acadrust writes them for R13-R2007/R13-R14 only), so
        // only DXF must round-trip those three.
        let assert_saved = |document: &CadDocument, is_dxf: bool| {
            let Some(EntityType::Leader(leader)) = document.get_entity(leader_handle) else {
                panic!("Leader missing after reopen");
            };
            assert_eq!(leader.vertices.len(), 4);
            if is_dxf {
                // BLOCKER: acadrust's DXF writer never emits group 340, so a
                // DXF save unlinks the annotation. Flip this to
                // `assert_eq!(.., text_handle)` once the engine writes it.
                assert!(leader.annotation_handle.is_null(), "acadrust now writes DXF 340; drop the blocker");
            } else {
                assert_eq!(leader.annotation_handle, text_handle);
            }
            assert!(!leader.arrow_enabled);
            // BLOCKER: neither writer stores `override_color`, so it reopens
            // as ByLayer in both formats.
            assert_eq!(leader.override_color, acadrust::types::Color::ByLayer,
                "acadrust now persists Leader.override_color; drop the blocker");
            if is_dxf {
                // BLOCKER: the DXF writer emits group 213 but the reader's
                // coordinate mapping does not reassemble it.
                assert_eq!(leader.annotation_offset, acadrust::types::Vector3::ZERO,
                    "acadrust now reads DXF 213; drop the blocker");
            } else {
                assert_eq!(leader.annotation_offset, acadrust::types::Vector3::new(1.0, 2.0, 0.0));
            }
            assert_eq!(leader.dimension_style, "Standard");
            if is_dxf {
                assert!((leader.text_height - 4.0).abs() < 1e-12);
                assert!(leader.hookline_enabled);
            } else {
                assert_eq!(leader.text_height, 2.5, "DWG R2010+ does not store text_height");
                assert!(!leader.hookline_enabled, "DWG R2010+ does not store hookline_enabled");
            }
            assert!(matches!(document.get_entity(text_handle), Some(EntityType::Text(_))));
        };
        let reopened_dwg = crate::io::load_bytes("leader.dwg",
            acadrust::DwgWriter::write_to_vec(host.document()).unwrap()).unwrap();
        let reopened_dxf = crate::io::load_bytes("leader.dxf",
            acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()).unwrap();
        assert_saved(&reopened_dwg, false);
        assert_saved(&reopened_dxf, true);
        for (name, mut document) in [("leader-reedit.dwg", reopened_dwg),
            ("leader-reedit.dxf", reopened_dxf)] {
            assert_geometry(document.get_entity(leader_handle).unwrap(), &document);
            let before = document.get_entity(leader_handle).unwrap().clone();
            let mut after = before.clone();
            let EntityType::Leader(leader) = &mut after else { unreachable!() };
            leader.vertices[3] = acadrust::types::Vector3::new(35.0, 12.0, 0.0);
            ocs_plugin_api::entity_coverage::validate_entity_mutation(&before, &after).unwrap();
            ocs_plugin_api::entity_coverage::validate_canvas_entity_references(&document, &after).unwrap();
            *document.get_entity_mut(leader_handle).unwrap() = after;
            let bytes = if name.ends_with("dwg") {
                acadrust::DwgWriter::write_to_vec(&document).unwrap()
            } else {
                acadrust::DxfWriter::new(&document).write_to_vec().unwrap()
            };
            assert!(matches!(crate::io::load_bytes(name, bytes).unwrap().get_entity(leader_handle),
                Some(EntityType::Leader(leader)) if (leader.vertices[3].x - 35.0).abs() < 1e-9));
        }

        // Erasing a leader also erases its annotation (scene leader grouping).
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.active_document.delete_entity({})", leader_handle.value()));
        assert!(host.document().get_entity(leader_handle).is_none());
        assert!(host.document().get_entity(text_handle).is_none());
        for (name, bytes) in [
            ("leader-deleted.dwg", acadrust::DwgWriter::write_to_vec(host.document()).unwrap()),
            ("leader-deleted.dxf", acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()),
        ] {
            let document = crate::io::load_bytes(name, bytes).unwrap();
            assert!(document.get_entity(leader_handle).is_none());
            assert!(document.get_entity(text_handle).is_none());
        }
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        // Text create, Leader create, edit transaction, Leader delete.
        assert_eq!(app.tabs[0].history.undo_stack.len(), 4);
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(leader_handle), Some(&expected));
        assert!(app.tabs[0].scene.document.get_entity(text_handle).is_some());
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(leader_handle), Some(&created));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(leader_handle).is_none());
        assert!(app.tabs[0].scene.document.get_entity(text_handle).is_some());
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(text_handle).is_none());
        app.redo_steps(4);
        assert!(app.tabs[0].scene.document.get_entity(leader_handle).is_none());
        assert!(app.tabs[0].scene.document.get_entity(text_handle).is_none());
    }

    #[test]
    fn staged_python_mline_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .unwrap();
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process.dispatch(host, command, &mut |_| {}).expect("Python dispatch"));
        };
        let last_output = |host: &HostSession<'_>| host.app.command_line.history.last().unwrap().text.clone();
        let style_handle = host.document().objects.iter().find_map(|(handle, object)| match object {
            acadrust::objects::ObjectType::MLineStyle(style) if style.name == "Standard" => Some(*handle),
            _ => None,
        }).expect("document carries the Standard MLineStyle");

        let script = std::env::temp_dir().join(format!("ocs_mline_create_{}.py", std::process::id()));
        std::fs::write(&script, concat!(
            "def p(x, y): return {'position': {'x': x, 'y': y, 'z': 0.0}}\n",
            "doc = ocs.active_document\n",
            "doc.create_entity('MLine', vertices=[p(0.0, 0.0), p(10.0, 0.0)])\n",
        )).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let handle = host.document().entities().find_map(|entity| match entity {
            EntityType::MLine(mline) => Some(mline.common.handle),
            _ => None,
        }).unwrap_or_else(|| panic!("Python did not create MLine: {}", last_output(&host)));
        let created = host.document().get_entity(handle).unwrap().clone();
        let EntityType::MLine(created_mline) = &created else { unreachable!() };
        assert_eq!(created_mline.style_handle, Some(style_handle));
        assert_eq!(created_mline.style_element_count, 2);
        assert_eq!(created_mline.start_point, acadrust::types::Vector3::ZERO);
        let first_parameters = |mline: &acadrust::entities::MLine, vertex: usize| -> Vec<f64> {
            mline.vertices[vertex].segments.iter().map(|segment| segment.parameters[0]).collect()
        };
        assert_eq!(first_parameters(created_mline, 0), vec![0.5, -0.5]);
        assert!(created_mline.vertices.iter().all(|vertex| {
            (vertex.direction.x - 1.0).abs() < 1e-9 && (vertex.miter.y.abs() - 1.0).abs() < 1e-9
        }));
        let assert_lines = |entity: &EntityType, document: &CadDocument, expected_vertices: usize| {
            let EntityType::MLine(mline) = entity else { panic!("expected MLine") };
            assert_eq!(mline.vertices.len(), expected_vertices);
            let lines = crate::entities::mline::mline_lines(mline, document);
            assert!(lines.len() >= 2, "expected styled element lines");
            assert!(lines.iter().flat_map(|line| &line.points).flatten().all(|v| v.is_finite()));
        };
        assert_lines(&created, host.document(), 2);

        let script = std::env::temp_dir().join(format!("ocs_mline_edit_{}.py", std::process::id()));
        std::fs::write(&script, format!(concat!(
            "def p(x, y): return {{'position': {{'x': x, 'y': y, 'z': 0.0}}}}\n",
            "doc = ocs.active_document\n",
            "mline = doc.entities[{}]\n",
            "with doc.transaction('Edit mline'):\n",
            "    mline.vertices = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)]\n",
            "    mline.scale_factor = 2.0\n",
            "    mline.justification = 'Top'\n",
            "doc.selection = [mline]\n"
        ), handle.value())).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let expected = host.document().get_entity(handle).unwrap().clone();
        let EntityType::MLine(edited) = &expected else { unreachable!() };
        assert_eq!(edited.vertices.len(), 3, "edit failed: {}", last_output(&host));
        assert_eq!(edited.scale_factor, 2.0);
        assert_eq!(edited.justification, acadrust::entities::MLineJustification::Top);
        assert_eq!(edited.style_handle, created_mline.style_handle);
        assert_eq!(edited.common, created_mline.common);
        assert_eq!(edited.normal, created_mline.normal);
        assert_eq!(edited.flags.bits() & !1, 0);
        // Top justification with scale 2: first element sits on the path,
        // the second 2.0 to the right of travel; the corner miter widens it.
        let end = first_parameters(edited, 0);
        assert!(end[0].abs() < 1e-9 && (end[1] + 2.0).abs() < 1e-9, "{end:?}");
        let corner = first_parameters(edited, 1);
        assert!(corner[1].abs() > 2.0 && corner[1].is_finite(), "miter must widen at the corner: {corner:?}");
        assert!((edited.vertices[1].direction.y - edited.vertices[1].direction.x).abs() > 1e-6);
        assert_lines(&expected, host.document(), 3);
        assert_eq!(host.selection(), vec![handle]);

        for (patch, message) in [
            ("'style_name':'Missing'", "does not exist"),
            ("'vertices':[{'position':{'x':0.0,'y':0.0,'z':0.0}}]", "at least 2"),
            ("'scale_factor':0.0", "nonzero"),
            ("'flags':64", "unknown bits"),
            ("'flags':3,'vertices':[{'position':{'x':0.0,'y':0.0,'z':0.0}},{'position':{'x':1.0,'y':0.0,'z':0.0}}]", "requires at least 3"),
            ("'vertices':[{'position':{'x':0.0,'y':0.0,'z':0.0}},{'position':{'x':0.0,'y':0.0,'z':0.0}}]", "coincide"),
            ("'vertices':[{'position':{'x':0.0,'y':0.0,'z':0.0}},{'position':{'x':1.0,'y':float('nan'),'z':0.0}}]", "finite"),
            ("'vertices':[{'position':{'x':0.0,'y':0.0,'z':0.0},'segments':[{}]},{'position':{'x':1.0,'y':0.0,'z':0.0}}]", "segments but style"),
            ("'start_point':{'x':5.0,'y':5.0,'z':0.0}", "read-only"),
        ] {
            dispatch(&mut host, &format!(
                "PY_EVAL ocs.update_many('Reject mline', [{{'handle':{}, {patch}}}])", handle.value()));
            assert_eq!(host.document().get_entity(handle), Some(&expected), "{patch}");
            assert!(last_output(&host).contains(message), "{patch}: {}", last_output(&host));
        }
        dispatch(&mut host, &format!(concat!(
            "PY_EVAL ocs.update_many('Atomic', [{{'handle':{},'scale_factor':3.0}},",
            "{{'handle':{},'scale_factor':0.0}}])"
        ), handle.value(), handle.value()));
        assert_eq!(host.document().get_entity(handle), Some(&expected));

        let assert_saved = |document: &CadDocument| {
            let Some(EntityType::MLine(mline)) = document.get_entity(handle) else {
                panic!("MLine missing after reopen");
            };
            assert_eq!(mline.vertices.len(), 3);
            assert_eq!(mline.scale_factor, 2.0);
            assert_eq!(mline.justification, acadrust::entities::MLineJustification::Top);
            assert_eq!(mline.style_name, "Standard");
            assert_eq!(mline.vertices[2].position, acadrust::types::Vector3::new(10.0, 10.0, 0.0));
            let (a, b) = (first_parameters(mline, 1), first_parameters(edited, 1));
            assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-9), "{a:?} vs {b:?}");
            assert_lines(&document.get_entity(handle).unwrap().clone(), document, 3);
        };
        let reopened_dwg = crate::io::load_bytes("mline.dwg",
            acadrust::DwgWriter::write_to_vec(host.document()).unwrap()).unwrap();
        let reopened_dxf = crate::io::load_bytes("mline.dxf",
            acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()).unwrap();
        assert_saved(&reopened_dwg);
        assert_saved(&reopened_dxf);
        for (name, mut document) in [("mline-reedit.dwg", reopened_dwg), ("mline-reedit.dxf", reopened_dxf)] {
            let before = document.get_entity(handle).unwrap().clone();
            let mut after = before.clone();
            let EntityType::MLine(mline) = &mut after else { unreachable!() };
            mline.vertices[2].position = acadrust::types::Vector3::new(10.0, 12.0, 0.0);
            ocs_plugin_api::entity_coverage::validate_entity_mutation(&before, &after).unwrap();
            ocs_plugin_api::entity_coverage::validate_canvas_entity_references(&document, &after).unwrap();
            let EntityType::MLine(mline) = &mut after else { unreachable!() };
            let old = match &before { EntityType::MLine(old) => old, _ => unreachable!() };
            crate::entities::mline::normalize_scripted_mline(Some(old), mline, &document).unwrap();
            *document.get_entity_mut(handle).unwrap() = after;
            let bytes = if name.ends_with("dwg") {
                acadrust::DwgWriter::write_to_vec(&document).unwrap()
            } else {
                acadrust::DxfWriter::new(&document).write_to_vec().unwrap()
            };
            assert!(matches!(crate::io::load_bytes(name, bytes).unwrap().get_entity(handle),
                Some(EntityType::MLine(mline)) if (mline.vertices[2].position.y - 12.0).abs() < 1e-9));
        }

        dispatch(&mut host, &format!("PY_EVAL ocs.active_document.delete_entity({})", handle.value()));
        assert!(host.document().get_entity(handle).is_none());
        for (name, bytes) in [
            ("mline-deleted.dwg", acadrust::DwgWriter::write_to_vec(host.document()).unwrap()),
            ("mline-deleted.dxf", acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()),
        ] {
            assert!(crate::io::load_bytes(name, bytes).unwrap().get_entity(handle).is_none());
        }
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle), Some(&expected));
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle), Some(&created));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    #[test]
    fn staged_python_dimension_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .unwrap();
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process.dispatch(host, command, &mut |_| {}).expect("Python dispatch"));
        };
        let last_output = |host: &HostSession<'_>| host.app.command_line.history.last().unwrap().text.clone();

        // (subtype, python fields, expected measurement)
        let fixtures: [(&str, &str, f64); 9] = [
            ("Linear", "definition_point=P(10,5), first_point=P(0,0), second_point=P(10,0)", 10.0),
            ("Aligned", "definition_point=P(1,5.5), first_point=P(0,0), second_point=P(3,4)", 5.0),
            ("Radius", "definition_point=P(5,0), angle_vertex=P(0,0)", 5.0),
            ("Diameter", "definition_point=P(10,0), angle_vertex=P(0,0)", 10.0),
            ("Angular3Pt", "definition_point=P(3,3), first_point=P(5,0), second_point=P(0,5), angle_vertex=P(0,0)", 90.0),
            ("Angular2Ln", "definition_point=P(0,5), dimension_arc=P(3,3), first_point=P(0,0), second_point=P(5,0), angle_vertex=P(0,0)", 90.0),
            ("Ordinate", "feature_location=P(5,3), leader_endpoint=P(5,8), is_ordinate_type_x=True", 5.0),
            ("Arc", "definition_point=P(3.5,3.5), first_extension_point=P(5,0), second_extension_point=P(0,5), center_point=P(0,0), arc_start_parameter=0.0, arc_end_parameter=1.5707963267948966", 5.0 * std::f64::consts::FRAC_PI_2),
            ("LargeRadial", "definition_point=P(10,0), chord_point=P(4,0)", 6.0),
        ];
        let mut script = String::from("def P(x, y): return {'x': x, 'y': y, 'z': 0.0}\ndoc = ocs.active_document\n");
        for (subtype, fields, _) in &fixtures {
            script.push_str(&format!("doc.create_entity('Dimension', subtype='{subtype}', {fields})\n"));
        }
        let path = std::env::temp_dir().join(format!("ocs_dimension_create_{}.py", std::process::id()));
        std::fs::write(&path, script).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", path.display()));
        let _ = std::fs::remove_file(&path);
        let subtype_of = |dimension: &acadrust::entities::Dimension| -> &'static str {
            use acadrust::entities::Dimension as D;
            match dimension {
                D::Aligned(_) => "Aligned", D::Linear(_) => "Linear", D::Radius(_) => "Radius",
                D::Diameter(_) => "Diameter", D::Angular2Ln(_) => "Angular2Ln",
                D::Angular3Pt(_) => "Angular3Pt", D::Ordinate(_) => "Ordinate",
                D::Arc(_) => "Arc", D::LargeRadial(_) => "LargeRadial",
            }
        };
        let handles_by_subtype = |document: &CadDocument| {
            let mut map = std::collections::BTreeMap::new();
            for entity in document.entities() {
                if let EntityType::Dimension(dimension) = entity {
                    map.insert(subtype_of(dimension), dimension.base().common.handle);
                }
            }
            map
        };
        let handles = handles_by_subtype(host.document());
        assert_eq!(handles.len(), 9, "created: {handles:?}; {}", last_output(&host));
        let measured = |document: &CadDocument, handle: Handle| -> f64 {
            let Some(EntityType::Dimension(dimension)) = document.get_entity(handle) else {
                panic!("Dimension missing");
            };
            dimension.base().actual_measurement
        };
        for (subtype, _, expected) in &fixtures {
            let value = measured(host.document(), handles[subtype]);
            assert!((value - expected).abs() < 1e-6, "{subtype}: measured {value}, expected {expected}");
            let Some(EntityType::Dimension(dimension)) = host.document().get_entity(handles[subtype]) else { unreachable!() };
            assert_eq!(dimension.measurement(), value, "{subtype}: stored measurement is derived");
            let wires = host.app.tabs[0].scene.wire_models_for(&[handles[subtype]]);
            assert!(!wires.is_empty(), "{subtype}: dimension produced no canvas geometry");
        }
        let linear = handles["Linear"];
        let created_linear = host.document().get_entity(linear).unwrap().clone();

        // Edit: move a defining point (the measurement follows), change text
        // and style-independent placement; untouched fields survive.
        let script = std::env::temp_dir().join(format!("ocs_dimension_edit_{}.py", std::process::id()));
        std::fs::write(&script, format!(concat!(
            "doc = ocs.active_document\n",
            "linear = doc.entities[{}]\n",
            "radius = doc.entities[{}]\n",
            "with doc.transaction('Edit dimensions'):\n",
            "    linear.second_point = (20.0, 0.0, 0.0)\n",
            "    linear.text = '<> mm'\n",
            "    linear.attachment_point = 'TopCenter'\n",
            "    radius.definition_point = (8.0, 0.0, 0.0)\n",
            "doc.selection = [linear]\n"
        ), linear.value(), handles["Radius"].value())).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let expected_linear = host.document().get_entity(linear).unwrap().clone();
        let EntityType::Dimension(edited) = &expected_linear else { unreachable!() };
        let EntityType::Dimension(before) = &created_linear else { unreachable!() };
        assert!((edited.base().actual_measurement - 20.0).abs() < 1e-9, "edit failed: {}", last_output(&host));
        assert!((measured(host.document(), handles["Radius"]) - 8.0).abs() < 1e-9);
        assert_eq!(edited.base().text, "<> mm");
        assert_eq!(edited.base().attachment_point, acadrust::entities::AttachmentPointType::TopCenter);
        assert_eq!(edited.base().common, before.base().common);
        assert_eq!(edited.base().style_name, before.base().style_name);
        assert_eq!(edited.base().normal, before.base().normal);
        assert_eq!(host.selection(), vec![linear]);
        let expected_radius = host.document().get_entity(handles["Radius"]).unwrap().clone();
        assert!(!host.app.tabs[0].scene.wire_models_for(&[linear]).is_empty());

        let reject_cases: [(&str, &str, &str); 9] = [
            ("Linear", "'leader_length':2.0", "does not apply to the Linear subtype"),
            ("Linear", "'subtype':'Radius'", "read-only"),
            ("Linear", "'actual_measurement':5.0", "read-only"),
            ("Linear", "'style_name':'Missing'", "does not exist"),
            ("Linear", "'first_point':{'x':20.0,'y':0.0,'z':0.0}", "must not coincide"),
            ("Linear", "'first_point':{'x':float('nan'),'y':0.0,'z':0.0}", "finite"),
            ("Linear", "'normal':{'x':0.0,'y':0.0,'z':0.0}", "nonzero"),
            ("Radius", "'angle_vertex':{'x':8.0,'y':0.0,'z':0.0}", "must not coincide"),
            ("Linear", "'attachment_point':'Nowhere'", "unsupported AttachmentPointType"),
        ];
        for (subtype, patch, message) in reject_cases {
            dispatch(&mut host, &format!(
                "PY_EVAL ocs.update_many('Reject dimension', [{{'handle':{}, {patch}}}])",
                handles[subtype].value()));
            assert_eq!(host.document().get_entity(linear), Some(&expected_linear), "{patch}");
            assert_eq!(host.document().get_entity(handles["Radius"]), Some(&expected_radius), "{patch}");
            assert!(last_output(&host).contains(message), "{patch}: {}", last_output(&host));
        }
        dispatch(&mut host, &format!(concat!(
            "PY_EVAL ocs.update_many('Atomic', [{{'handle':{},'text':'changed'}},",
            "{{'handle':{},'style_name':'Missing'}}])"
        ), linear.value(), handles["Radius"].value()));
        assert_eq!(host.document().get_entity(linear), Some(&expected_linear));
        for (patch, message) in [
            ("ocs.active_document.create_entity('Dimension', definition_point=P(0,0))", "requires subtype"),
            ("ocs.active_document.create_entity('Dimension', subtype='Cone')", "unsupported Dimension subtype"),
            ("ocs.active_document.create_entity('Dimension', subtype='Linear', first_point=P(0,0), second_point=P(1,0))", "requires definition_point"),
        ] {
            dispatch(&mut host, &format!("PY_EVAL (lambda P: {patch})(lambda x, y: {{'x': x, 'y': y, 'z': 0.0}})"));
            assert_eq!(handles_by_subtype(host.document()).len(), 9, "{patch}");
            assert!(last_output(&host).contains(message), "{patch}: {}", last_output(&host));
        }

        // DWG and DXF: every subtype is recorded separately.
        let mut failures = Vec::new();
        let dwg = crate::io::load_bytes("dimension.dwg", acadrust::DwgWriter::write_to_vec(host.document()).unwrap()).unwrap();
        let dxf = crate::io::load_bytes("dimension.dxf", acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()).unwrap();
        for (format, document) in [("DWG", &dwg), ("DXF", &dxf)] {
            let reopened = handles_by_subtype(document);
            for (subtype, _, expected) in &fixtures {
                let expected = match *subtype { "Linear" => 20.0, "Radius" => 8.0, _ => *expected };
                match reopened.get(subtype) {
                    None => failures.push(format!("{format} {subtype}: missing")),
                    Some(handle) => {
                        let value = measured(document, *handle);
                        if (value - expected).abs() > 1e-6 {
                            failures.push(format!("{format} {subtype}: measurement {value} != {expected}"));
                        }
                    }
                }
            }
            let Some(EntityType::Dimension(dimension)) = document.get_entity(linear) else {
                failures.push(format!("{format} Linear handle lost"));
                continue;
            };
            if dimension.base().text != "<> mm" { failures.push(format!("{format} Linear text {:?}", dimension.base().text)); }
        }
        assert!(failures.is_empty(), "persistence gaps: {failures:#?}");
        for (name, mut document) in [("dimension-reedit.dwg", dwg), ("dimension-reedit.dxf", dxf)] {
            let before = document.get_entity(linear).unwrap().clone();
            let mut after = before.clone();
            let EntityType::Dimension(dimension) = &mut after else { unreachable!() };
            if let acadrust::entities::Dimension::Linear(value) = dimension { value.second_point.x = 30.0; }
            ocs_plugin_api::entity_coverage::validate_entity_mutation(&before, &after).unwrap();
            ocs_plugin_api::entity_coverage::validate_canvas_entity_references(&document, &after).unwrap();
            let EntityType::Dimension(dimension) = &mut after else { unreachable!() };
            crate::entities::dimension::normalize_scripted_dimension(dimension);
            *document.get_entity_mut(linear).unwrap() = after;
            let bytes = if name.ends_with("dwg") {
                acadrust::DwgWriter::write_to_vec(&document).unwrap()
            } else {
                acadrust::DxfWriter::new(&document).write_to_vec().unwrap()
            };
            let reopened = crate::io::load_bytes(name, bytes).unwrap();
            assert!((measured(&reopened, linear) - 30.0).abs() < 1e-6, "{name}");
        }

        dispatch(&mut host, &format!("PY_EVAL ocs.active_document.delete_entity({})", linear.value()));
        assert!(host.document().get_entity(linear).is_none());
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(linear), Some(&expected_linear));
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(linear), Some(&created_linear));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(linear).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(linear).is_none());
    }

    #[test]
    fn staged_python_multileader_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .unwrap();
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process.dispatch(host, command, &mut |_| {}).expect("Python dispatch"));
        };
        let last_output = |host: &HostSession<'_>| host.app.command_line.history.last().unwrap().text.clone();

        let script = std::env::temp_dir().join(format!("ocs_mleader_create_{}.py", std::process::id()));
        std::fs::write(&script, concat!(
            "def P(x, y): return {'x': x, 'y': y, 'z': 0.0}\n",
            "doc = ocs.active_document\n",
            "doc.create_entity('MultiLeader', content_type='MText', context={\n",
            "    'has_text_contents': True, 'text_string': 'NOTE', 'text_location': P(20, 10),\n",
            "    'text_height': 2.5,\n",
            "    'leader_roots': [{'connection_point': P(15, 10), 'direction': P(1, 0), 'landing_distance': 2.0,\n",
            "        'lines': [{'points': [P(0, 0), P(10, 10), P(15, 10)]}]}]})\n",
        )).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let handle = host.document().entities().find_map(|entity| match entity {
            EntityType::MultiLeader(value) => Some(value.common.handle),
            _ => None,
        }).unwrap_or_else(|| panic!("Python did not create MultiLeader: {}", last_output(&host)));
        let created = host.document().get_entity(handle).unwrap().clone();
        let EntityType::MultiLeader(created_ml) = &created else { unreachable!() };
        assert_eq!(created_ml.context.text_string, "NOTE");
        assert_eq!(created_ml.context.leader_roots[0].lines[0].points.len(), 3);
        assert!(!host.app.tabs[0].scene.wire_models_for(&[handle]).is_empty(), "no canvas geometry");

        let script = std::env::temp_dir().join(format!("ocs_mleader_edit_{}.py", std::process::id()));
        std::fs::write(&script, format!(concat!(
            "def P(x, y): return {{'x': x, 'y': y, 'z': 0.0}}\n",
            "doc = ocs.active_document\n",
            "ml = doc.entities[{}]\n",
            "roots = ml.context['leader_roots']\n",
            "roots[0]['lines'][0]['points'] = [P(0, 0), P(12, 12), P(18, 12), P(22, 12)]\n",
            "with doc.transaction('Edit multileader'):\n",
            "    ml.context = {{'leader_roots': roots, 'text_string': 'EDITED', 'text_location': P(26, 12), 'text_height': 3.0}}\n",
            "    ml.dogleg_length = 3.0\n",
            "    ml.line_color = {{'kind': 'Rgb', 'value': {{'r': 0, 'g': 128, 'b': 255}}}}\n",
            "doc.selection = [ml]\n"
        ), handle.value())).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let expected = host.document().get_entity(handle).unwrap().clone();
        let EntityType::MultiLeader(edited) = &expected else { unreachable!() };
        assert_eq!(edited.context.text_string, "EDITED", "edit failed: {}", last_output(&host));
        assert_eq!(edited.context.leader_roots[0].lines[0].points.len(), 4);
        assert_eq!(edited.dogleg_length, 3.0);
        assert_eq!(edited.context.text_height, 3.0);
        assert_eq!(edited.text_height, created_ml.text_height);
        assert_eq!(edited.line_color, acadrust::types::Color::Rgb { r: 0, g: 128, b: 255 });
        // Untouched context fields and the entity identity survive a partial patch.
        assert_eq!(edited.common, created_ml.common);
        assert_eq!(edited.context.scale_factor, created_ml.context.scale_factor);
        assert_eq!(edited.content_type, created_ml.content_type);
        assert_eq!(edited.context.leader_roots[0].connection_point, created_ml.context.leader_roots[0].connection_point);
        assert_eq!(host.selection(), vec![handle]);
        assert!(!host.app.tabs[0].scene.wire_models_for(&[handle]).is_empty());

        for (patch, message) in [
            ("'text_height':0.0", "read-only"),
            ("'scale_factor':-1.0", "greater than zero"),
            ("'context':{'text_height':float('inf')}", "finite"),
            ("'style_handle':999999", "does not exist"),
            ("'text_style_handle':999999", "does not exist"),
            ("'content_type':'None'", "cannot carry"),
            ("'context':{'nonsense':1}", "outside the editable schema"),
            ("'context':{'leader_roots':[{'lines':[{'points':[]}]}]}", "has no points"),
            ("'context':{'text_location':{'x':float('nan'),'y':0.0,'z':0.0}}", "finite"),
            ("'dwg_version':5", "read-only"),
            ("'line_color':{'kind':'Index','value':300}", "Color"),
        ] {
            dispatch(&mut host, &format!(
                "PY_EVAL ocs.update_many('Reject multileader', [{{'handle':{}, {patch}}}])", handle.value()));
            assert_eq!(host.document().get_entity(handle), Some(&expected), "{patch}");
            assert!(last_output(&host).contains(message), "{patch}: {}", last_output(&host));
        }
        dispatch(&mut host, &format!(concat!(
            "PY_EVAL ocs.update_many('Atomic', [{{'handle':{},'dogleg_length':9.0}},",
            "{{'handle':{},'scale_factor':-1.0}}])"
        ), handle.value(), handle.value()));
        assert_eq!(host.document().get_entity(handle), Some(&expected));

        let mut gaps = Vec::new();
        let dwg = crate::io::load_bytes("mleader.dwg", acadrust::DwgWriter::write_to_vec(host.document()).unwrap()).unwrap();
        let dxf = crate::io::load_bytes("mleader.dxf", acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()).unwrap();
        for (format, document) in [("DWG", &dwg), ("DXF", &dxf)] {
            let Some(EntityType::MultiLeader(value)) = document.get_entity(handle) else {
                gaps.push(format!("{format}: MultiLeader missing"));
                continue;
            };
            if value.context.text_string != "EDITED" { gaps.push(format!("{format}: text {:?}", value.context.text_string)); }
            let points = value.context.leader_roots.first().and_then(|r| r.lines.first()).map_or(0, |l| l.points.len());
            if points != 4 { gaps.push(format!("{format}: leader points {points}")); }
            if value.dogleg_length != 3.0 { gaps.push(format!("{format}: dogleg {}", value.dogleg_length)); }
            if value.context.text_height != edited.context.text_height { gaps.push(format!("{format}: context.text_height {}", value.context.text_height)); }
            if value.line_color != edited.line_color { gaps.push(format!("{format}: line_color {:?}", value.line_color)); }
        }
        assert!(gaps.is_empty(), "persistence gaps: {gaps:#?}");
        for (name, mut document) in [("mleader-reedit.dwg", dwg), ("mleader-reedit.dxf", dxf)] {
            let before = document.get_entity(handle).unwrap().clone();
            let mut after = before.clone();
            let EntityType::MultiLeader(value) = &mut after else { unreachable!() };
            value.dogleg_length = 4.5;
            ocs_plugin_api::entity_coverage::validate_entity_mutation(&before, &after).unwrap();
            ocs_plugin_api::entity_coverage::validate_canvas_entity_references(&document, &after).unwrap();
            *document.get_entity_mut(handle).unwrap() = after;
            let bytes = if name.ends_with("dwg") {
                acadrust::DwgWriter::write_to_vec(&document).unwrap()
            } else {
                acadrust::DxfWriter::new(&document).write_to_vec().unwrap()
            };
            assert!(matches!(crate::io::load_bytes(name, bytes).unwrap().get_entity(handle),
                Some(EntityType::MultiLeader(value)) if value.dogleg_length == 4.5), "{name}");
        }

        dispatch(&mut host, &format!("PY_EVAL ocs.active_document.delete_entity({})", handle.value()));
        assert!(host.document().get_entity(handle).is_none());
        for (name, bytes) in [
            ("mleader-deleted.dwg", acadrust::DwgWriter::write_to_vec(host.document()).unwrap()),
            ("mleader-deleted.dxf", acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()),
        ] {
            assert!(crate::io::load_bytes(name, bytes).unwrap().get_entity(handle).is_none());
        }
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle), Some(&expected));
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle), Some(&created));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    #[test]
    fn staged_python_table_lifecycle_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .unwrap();
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process.dispatch(host, command, &mut |_| {}).expect("Python dispatch"));
        };
        let last_output = |host: &HostSession<'_>| host.app.command_line.history.last().unwrap().text.clone();

        dispatch(&mut host, "PY_EVAL ocs.active_document.create_entity('Table', insertion_point={'x':5.0,'y':5.0,'z':0.0}).handle");
        let handle = host.document().entities().find_map(|entity| match entity {
            EntityType::Table(value) => Some(value.common.handle),
            _ => None,
        }).unwrap_or_else(|| panic!("Python did not create Table: {}", last_output(&host)));
        let created = host.document().get_entity(handle).unwrap().clone();
        let EntityType::Table(created_table) = &created else { unreachable!() };
        assert_eq!((created_table.rows.len(), created_table.columns.len()), (3, 3));
        assert_eq!(created_table.insertion_point, acadrust::types::Vector3::new(5.0, 5.0, 0.0));
        assert!(!host.app.tabs[0].scene.wire_models_for(&[handle]).is_empty(), "no canvas geometry");

        let script = std::env::temp_dir().join(format!("ocs_table_edit_{}.py", std::process::id()));
        std::fs::write(&script, format!(concat!(
            "doc = ocs.active_document\n",
            "t = doc.entities[{}]\n",
            "rows = t.rows\n",
            "cols = t.columns\n",
            "rows.append(dict(rows[0]))\n",
            "cols[0]['width'] = 30.0\n",
            "rows[1]['cells'][1]['contents'] = [{{'content_type': 'Value', 'scale': 1.0, 'text_height': 2.5,\n",
            "    'value': {{'value_type': 'String', 'raw_type_code': 4, 'text': 'Hello', 'formatted_value': 'Hello'}}}}]\n",
            "with doc.transaction('Edit table'):\n",
            "    t.rows = rows\n",
            "    t.columns = cols\n",
            "    t.merged_ranges = [{{'top_row': 0, 'left_col': 0, 'bottom_row': 0, 'right_col': 2}}]\n",
            "    t.break_spacing = 2.0\n",
            "doc.selection = [t]\n"
        ), handle.value())).unwrap();
        dispatch(&mut host, &format!("PY_RUN {}", script.display()));
        let _ = std::fs::remove_file(&script);
        let expected = host.document().get_entity(handle).unwrap().clone();
        let EntityType::Table(edited) = &expected else { unreachable!() };
        assert_eq!(edited.rows.len(), 4, "edit failed: {}", last_output(&host));
        assert_eq!(edited.columns[0].width, 30.0);
        assert_eq!(edited.rows[1].cells[1].contents[0].value.text, "Hello");
        assert_eq!(edited.merged_ranges.len(), 1);
        assert_eq!(edited.break_spacing, 2.0);
        assert_eq!(edited.common, created_table.common);
        assert_eq!(edited.insertion_point, created_table.insertion_point);
        assert_eq!(edited.normal, created_table.normal);
        assert_eq!(edited.columns[1], created_table.columns[1]);
        assert_eq!(edited.rows[2], created_table.rows[2]);
        assert_eq!(edited.table_style_handle, created_table.table_style_handle);
        assert_eq!(host.selection(), vec![handle]);
        assert!(!host.app.tabs[0].scene.wire_models_for(&[handle]).is_empty());

        for (patch, message) in [
            ("'columns':[{'name':'A','width':0.0},{'width':1.0},{'width':1.0}]", "greater than zero"),
            ("'columns':[{'width':5.0}]", "cells but the table has 1 columns"),
            ("'columns':[]", "at least one row and one column"),
            ("'merged_ranges':[{'top_row':0,'left_col':0,'bottom_row':9,'right_col':0}]", "outside the"),
            ("'merged_ranges':[{'top_row':1,'left_col':0,'bottom_row':2,'right_col':1},{'top_row':2,'left_col':1,'bottom_row':3,'right_col':2}]", "overlaps"),
            ("'table_style_handle':999999", "does not exist"),
            ("'block_name':'x'", "read-only"),
            ("'normal':{'x':0.0,'y':0.0,'z':0.0}", "nonzero"),
            ("'break_spacing':-1.0", "non-negative"),
            ("'insertion_point':{'x':float('nan'),'y':0.0,'z':0.0}", "finite"),
        ] {
            dispatch(&mut host, &format!(
                "PY_EVAL ocs.update_many('Reject table', [{{'handle':{}, {patch}}}])", handle.value()));
            assert_eq!(host.document().get_entity(handle), Some(&expected), "{patch}");
            assert!(last_output(&host).contains(message), "{patch}: {}", last_output(&host));
        }
        dispatch(&mut host, &format!(concat!(
            "PY_EVAL ocs.update_many('Atomic', [{{'handle':{},'break_spacing':7.0}},",
            "{{'handle':{},'break_spacing':-1.0}}])"
        ), handle.value(), handle.value()));
        assert_eq!(host.document().get_entity(handle), Some(&expected));

        let mut gaps = Vec::new();
        let dwg = crate::io::load_bytes("table.dwg", acadrust::DwgWriter::write_to_vec(host.document()).unwrap()).unwrap();
        let dxf = crate::io::load_bytes("table.dxf", acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()).unwrap();
        for (format, document) in [("DWG", &dwg), ("DXF", &dxf)] {
            let Some(EntityType::Table(value)) = document.get_entity(handle) else {
                gaps.push(format!("{format}: Table missing"));
                continue;
            };
            if value.rows.len() != 4 { gaps.push(format!("{format}: rows {}", value.rows.len())); }
            if value.columns.first().map(|c| c.width) != Some(30.0) { gaps.push(format!("{format}: column width {:?}", value.columns.first().map(|c| c.width))); }
            let text = value.rows.get(1).and_then(|r| r.cells.get(1)).and_then(|c| c.contents.first()).map(|c| c.value.text.clone());
            if text.as_deref() != Some("Hello") { gaps.push(format!("{format}: cell text {text:?}")); }
            // DWG stores the ranges; DXF stores the origin cell's merge
            // dimensions and its reader does not rebuild `merged_ranges`
            // (engine gap, recorded in the ledger). Scripted saves populate
            // both, so each format keeps the merge in its own form.
            let origin = &value.rows[0].cells[0];
            if format == "DXF" {
                if (origin.merge_width, origin.merge_height) != (3, 1) { gaps.push(format!("DXF: merge dims {}x{}", origin.merge_width, origin.merge_height)); }
                if !value.merged_ranges.is_empty() { gaps.push("DXF: reader now rebuilds merged_ranges; drop the blocker".into()); }
            } else if value.merged_ranges.len() != 1 {
                gaps.push(format!("DWG: merged_ranges {}", value.merged_ranges.len()));
            }
            if value.insertion_point != edited.insertion_point { gaps.push(format!("{format}: insertion {:?}", value.insertion_point)); }
        }
        assert!(gaps.is_empty(), "persistence gaps: {gaps:#?}");
        for (name, mut document) in [("table-reedit.dwg", dwg), ("table-reedit.dxf", dxf)] {
            let before = document.get_entity(handle).unwrap().clone();
            let mut after = before.clone();
            let EntityType::Table(value) = &mut after else { unreachable!() };
            value.columns[1].width = 12.0;
            ocs_plugin_api::entity_coverage::validate_entity_mutation(&before, &after).unwrap();
            ocs_plugin_api::entity_coverage::validate_canvas_entity_references(&document, &after).unwrap();
            *document.get_entity_mut(handle).unwrap() = after;
            let bytes = if name.ends_with("dwg") {
                acadrust::DwgWriter::write_to_vec(&document).unwrap()
            } else {
                acadrust::DxfWriter::new(&document).write_to_vec().unwrap()
            };
            assert!(matches!(crate::io::load_bytes(name, bytes).unwrap().get_entity(handle),
                Some(EntityType::Table(value)) if value.columns[1].width == 12.0), "{name}");
        }

        dispatch(&mut host, &format!("PY_EVAL ocs.active_document.delete_entity({})", handle.value()));
        assert!(host.document().get_entity(handle).is_none());
        for (name, bytes) in [
            ("table-deleted.dwg", acadrust::DwgWriter::write_to_vec(host.document()).unwrap()),
            ("table-deleted.dxf", acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()),
        ] {
            assert!(crate::io::load_bytes(name, bytes).unwrap().get_entity(handle).is_none());
        }
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle), Some(&expected));
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle), Some(&created));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    /// One scripted lifecycle shared by the mesh kinds: create, edit,
    /// reject, save/reopen both formats, re-edit, delete, undo/redo.
    struct MeshCase {
        create: &'static str,
        edit: &'static str,
        rejects: &'static [(&'static str, &'static str)],
        is_kind: fn(&EntityType) -> bool,
        digest: fn(&EntityType) -> String,
        reedit: fn(&mut EntityType),
        expect_created: &'static str,
        expect_edited: &'static str,
        expect_reedited: &'static str,
        /// Digest a DXF reopen yields when the engine loses a field (empty:
        /// same as DWG). A canary that fails once the engine round-trips it.
        expect_edited_dxf: &'static str,
        expect_reedited_dxf: &'static str,
    }

    fn run_mesh_lifecycle(case: &MeshCase) {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        let process = ocs_plugin_api::process::PluginProcess::spawn(
            std::path::Path::new(&plugin_path),
            &mut host,
            crate::plugin::v4_support::notification_handler(),
        )
        .unwrap();
        let dispatch = |host: &mut HostSession<'_>, command: &str| {
            assert!(process.dispatch(host, command, &mut |_| {}).expect("Python dispatch"));
        };
        let last_output = |host: &HostSession<'_>| host.app.command_line.history.last().unwrap().text.clone();
        let run_script = |host: &mut HostSession<'_>, tag: &str, body: &str| {
            let path = std::env::temp_dir().join(format!("ocs_mesh_{tag}_{}.py", std::process::id()));
            std::fs::write(&path, format!("def P(x, y, z): return {{'x': x, 'y': y, 'z': z}}\ndoc = ocs.active_document\n{body}")).unwrap();
            dispatch(host, &format!("PY_RUN {}", path.display()));
            let _ = std::fs::remove_file(&path);
        };

        let tmp = std::env::temp_dir().to_string_lossy().into_owned();
        let mut create_script = case.create.replace("TMPDIR", &tmp);
        let paper = host.document().block_records.iter().find(|r| r.is_paper_space()).map(|r| r.handle.value());
        let layer0 = host.document().layers.iter().find(|l| l.name == "0").map(|l| l.handle.value());
        create_script = create_script
            .replace("PAPERSPACE", &paper.unwrap_or_default().to_string())
            .replace("LAYER0", &layer0.unwrap_or_default().to_string());
        if create_script.contains("DEFHANDLE") {
            // Scripts reference an underlay definition that the drawing owns.
            let handle = host.document_mut().allocate_handle();
            let mut definition = acadrust::objects::UnderlayDefinition::pdf("plan.pdf", "1");
            definition.handle = handle;
            host.document_mut().objects.insert(handle, acadrust::objects::ObjectType::UnderlayDefinition(definition));
            create_script = create_script.replace("DEFHANDLE", &handle.value().to_string());
        }
        run_script(&mut host, "create", &create_script);
        let handle = host.document().entities().find(|entity| (case.is_kind)(entity))
            .map(|entity| entity.common().handle)
            .unwrap_or_else(|| panic!("Python did not create the mesh: {}", last_output(&host)));
        let created = host.document().get_entity(handle).unwrap().clone();
        assert_eq!((case.digest)(&created), case.expect_created);
        // Viewports draw only on their paper-space layout tab, which this
        // driver does not activate, so they have no model-tab wire oracle.
        let has_wires = !matches!(created, EntityType::Viewport(_));
        if has_wires {
            assert!(!host.app.tabs[0].scene.wire_models_for(&[handle]).is_empty(), "no canvas geometry");
        }

        run_script(&mut host, "edit", &case.edit.replace("HANDLE", &handle.value().to_string()).replace("TMPDIR", &tmp).replace("LAYER0", &layer0.unwrap_or_default().to_string()));
        let expected = host.document().get_entity(handle).unwrap().clone();
        assert_eq!((case.digest)(&expected), case.expect_edited, "edit failed: {}", last_output(&host));
        assert_eq!(expected.common(), created.common());
        assert_eq!(host.selection(), vec![handle]);
        if has_wires {
            assert!(!host.app.tabs[0].scene.wire_models_for(&[handle]).is_empty());
        }

        for (patch, message) in case.rejects {
            dispatch(&mut host, &format!(
                "PY_EVAL ocs.update_many('Reject mesh', [{{'handle':{}, {patch}}}])", handle.value()));
            assert_eq!(host.document().get_entity(handle), Some(&expected), "{patch}");
            assert!(last_output(&host).contains(message), "{patch}: {}", last_output(&host));
        }
        let (first_patch, _) = case.rejects[0];
        dispatch(&mut host, &format!(
            "PY_EVAL ocs.update_many('Atomic', [{{'handle':{h}, 'layer':'Other'}}, {{'handle':{h}, {first_patch}}}])",
            h = handle.value()));
        assert_eq!(host.document().get_entity(handle), Some(&expected));

        let dwg = crate::io::load_bytes("mesh.dwg", acadrust::DwgWriter::write_to_vec(host.document()).unwrap()).unwrap();
        let dxf = crate::io::load_bytes("mesh.dxf", acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()).unwrap();
        let mut gaps = Vec::new();
        for (format, document) in [("DWG", &dwg), ("DXF", &dxf)] {
            let expected_digest = if format == "DXF" && !case.expect_edited_dxf.is_empty() {
                case.expect_edited_dxf
            } else {
                case.expect_edited
            };
            match document.get_entity(handle) {
                Some(entity) if (case.digest)(entity) == expected_digest => {}
                Some(entity) => gaps.push(format!("{format}: {}", (case.digest)(entity))),
                None => gaps.push(format!("{format}: entity missing")),
            }
        }
        assert!(gaps.is_empty(), "persistence gaps (expected {} / DXF {:?}): {gaps:#?}", case.expect_edited, case.expect_edited_dxf);
        for (name, mut document) in [("mesh-reedit.dwg", dwg), ("mesh-reedit.dxf", dxf)] {
            let before = document.get_entity(handle).unwrap().clone();
            let mut after = before.clone();
            (case.reedit)(&mut after);
            ocs_plugin_api::entity_coverage::validate_entity_mutation(&before, &after).unwrap();
            ocs_plugin_api::entity_coverage::validate_canvas_entity_references(&document, &after).unwrap();
            *document.get_entity_mut(handle).unwrap() = after;
            let bytes = if name.ends_with("dwg") {
                acadrust::DwgWriter::write_to_vec(&document).unwrap()
            } else {
                acadrust::DxfWriter::new(&document).write_to_vec().unwrap()
            };
            let reopened = crate::io::load_bytes(name, bytes).unwrap();
            let expected_digest = if name.ends_with("dxf") && !case.expect_reedited_dxf.is_empty() {
                case.expect_reedited_dxf
            } else {
                case.expect_reedited
            };
            assert_eq!((case.digest)(reopened.get_entity(handle).unwrap()), expected_digest, "{name}");
        }

        dispatch(&mut host, &format!("PY_EVAL ocs.active_document.delete_entity({})", handle.value()));
        assert!(host.document().get_entity(handle).is_none());
        for (name, bytes) in [
            ("mesh-deleted.dwg", acadrust::DwgWriter::write_to_vec(host.document()).unwrap()),
            ("mesh-deleted.dxf", acadrust::DxfWriter::new(host.document()).write_to_vec().unwrap()),
        ] {
            assert!(crate::io::load_bytes(name, bytes).unwrap().get_entity(handle).is_none());
        }
        drop(process);
        drop(host);
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 3);
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle), Some(&expected));
        app.undo_steps(1);
        assert_eq!(app.tabs[0].scene.document.get_entity(handle), Some(&created));
        app.undo_steps(1);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
        app.redo_steps(3);
        assert!(app.tabs[0].scene.document.get_entity(handle).is_none());
    }

    #[test]
    fn staged_python_polygon_mesh_lifecycle_over_real_ipc() {
        run_mesh_lifecycle(&MeshCase {
            create: concat!(
                "verts = [{'location': P(i, j, 0.0)} for i in range(3) for j in range(3)]\n",
                "doc.create_entity('PolygonMesh', m_vertex_count=3, n_vertex_count=3, vertices=verts)\n"),
            edit: concat!(
                "mesh = doc.entities[HANDLE]\n",
                "verts = mesh.vertices\n",
                "verts[4]['location'] = P(1.0, 1.0, 2.5)\n",
                "with doc.transaction('Edit mesh'):\n",
                "    mesh.vertices = verts\n",
                "    mesh.m_smooth_density = 6\n",
                "    mesh.n_smooth_density = 6\n",
                "    mesh.smooth_type = 'Cubic'\n",
                "doc.selection = [mesh]\n"),
            rejects: &[
                ("'m_vertex_count':4", "requires 12"),
                ("'n_vertex_count':1", "at least 2"),
                ("'m_smooth_density':-1", "non-negative"),
                ("'elevation':float('nan')", "finite"),
                ("'normal':{'x':0.0,'y':0.0,'z':0.0}", "nonzero"),
                ("'vertices':[{'location':{'x':float('nan'),'y':0.0,'z':0.0}}]*9", "finite"),
                ("'smooth_type':'Spline'", "unsupported"),
            ],
            is_kind: |entity| matches!(entity, EntityType::PolygonMesh(_)),
            digest: |entity| match entity {
                EntityType::PolygonMesh(m) => format!("{}x{} v{} z{} d{}/{} {:?}", m.m_vertex_count, m.n_vertex_count,
                    m.vertices.len(), m.vertices[4].location.z, m.m_smooth_density, m.n_smooth_density, m.smooth_type),
                _ => "wrong kind".into(),
            },
            reedit: |entity| if let EntityType::PolygonMesh(m) = entity { m.vertices[0].location.z = 1.0; },
            expect_created: "3x3 v9 z0 d0/0 NoSmooth",
            expect_edited: "3x3 v9 z2.5 d6/6 Cubic",
            expect_reedited: "3x3 v9 z2.5 d6/6 Cubic",
            expect_edited_dxf: "",
            expect_reedited_dxf: "",
        });
    }

    #[test]
    fn staged_python_polyface_mesh_lifecycle_over_real_ipc() {
        run_mesh_lifecycle(&MeshCase {
            create: concat!(
                "verts = [{'location': P(0.0, 0.0, 0.0)}, {'location': P(4.0, 0.0, 0.0)},\n",
                "         {'location': P(4.0, 4.0, 0.0)}, {'location': P(0.0, 4.0, 0.0)}]\n",
                "faces = [{'index1': 1, 'index2': 2, 'index3': 3}, {'index1': 1, 'index2': 3, 'index3': 4}]\n",
                "doc.create_entity('PolyfaceMesh', vertices=verts, faces=faces)\n"),
            edit: concat!(
                "mesh = doc.entities[HANDLE]\n",
                "verts = mesh.vertices\n",
                "verts[2]['location'] = P(4.0, 4.0, 3.0)\n",
                "verts.append({'location': P(2.0, 6.0, 0.0)})\n",
                "faces = mesh.faces\n",
                "faces.append({'index1': 3, 'index2': -4, 'index3': 5})\n",
                "with doc.transaction('Edit mesh'):\n",
                "    mesh.vertices = verts\n",
                "    mesh.faces = faces\n",
                "doc.selection = [mesh]\n"),
            rejects: &[
                ("'faces':[{'index1':1,'index2':2,'index3':9}]", "references vertex 9"),
                ("'faces':[{'index1':1,'index2':2}]", "at least 3 vertex indices"),
                ("'faces':[]", "at least 1 face"),
                ("'vertices':[{'location':{'x':0.0,'y':0.0,'z':0.0}}]", "at least 3"),
                ("'thickness':float('inf')", "finite"),
                ("'seqend_handle':5", "read-only"),
            ],
            is_kind: |entity| matches!(entity, EntityType::PolyfaceMesh(_)),
            digest: |entity| match entity {
                EntityType::PolyfaceMesh(m) => format!("v{} f{} z{} last {:?}", m.vertices.len(), m.faces.len(),
                    m.vertices[2].location.z,
                    m.faces.last().map(|f| (f.index1, f.index2, f.index3))),
                _ => "wrong kind".into(),
            },
            reedit: |entity| if let EntityType::PolyfaceMesh(m) = entity { m.vertices[0].location.z = 1.0; },
            expect_created: "v4 f2 z0 last Some((1, 3, 4))",
            expect_edited: "v5 f3 z3 last Some((3, -4, 5))",
            expect_reedited: "v5 f3 z3 last Some((3, -4, 5))",
            expect_edited_dxf: "",
            expect_reedited_dxf: "",
        });
    }

    #[test]
    fn staged_python_mesh_lifecycle_over_real_ipc() {
        run_mesh_lifecycle(&MeshCase {
            create: concat!(
                "verts = [P(0.0, 0.0, 0.0), P(4.0, 0.0, 0.0), P(4.0, 4.0, 0.0), P(0.0, 4.0, 0.0)]\n",
                "doc.create_entity('Mesh', vertices=verts, faces=[{'vertices': [0, 1, 2, 3]}],\n",
                "    edges=[{'start': 0, 'end': 1, 'crease': 1.5}])\n"),
            edit: concat!(
                "mesh = doc.entities[HANDLE]\n",
                "verts = mesh.vertices\n",
                "verts.append(P(2.0, 6.0, 0.0))\n",
                "faces = mesh.faces\n",
                "faces.append({'vertices': [3, 2, 4]})\n",
                "with doc.transaction('Edit mesh'):\n",
                "    mesh.vertices = verts\n",
                "    mesh.faces = faces\n",
                "    mesh.subdivision_level = 1\n",
                "doc.selection = [mesh]\n"),
            rejects: &[
                ("'faces':[{'vertices':[0,1,9]}]", "references vertex 9"),
                ("'faces':[{'vertices':[0,1]}]", "at least 3 vertices"),
                ("'edges':[{'start':0,'end':9}]", "out of range"),
                ("'edges':[{'start':2,'end':2}]", "identical endpoints"),
                ("'edges':[{'start':0,'end':1,'crease':-1.0}]", "non-negative"),
                ("'subdivision_level':-1", "non-negative"),
                ("'version':3", "read-only"),
            ],
            is_kind: |entity| matches!(entity, EntityType::Mesh(_)),
            digest: |entity| match entity {
                EntityType::Mesh(m) => format!("v{} f{} e{} sub{} crease {:?}", m.vertices.len(), m.faces.len(),
                    m.edges.len(), m.subdivision_level, m.edges.first().and_then(|e| e.crease)),
                _ => "wrong kind".into(),
            },
            reedit: |entity| if let EntityType::Mesh(m) = entity { m.vertices[0].z = 1.0; },
            expect_created: "v4 f1 e1 sub0 crease Some(1.5)",
            expect_edited: "v5 f2 e1 sub1 crease Some(1.5)",
            expect_reedited: "v5 f2 e1 sub1 crease Some(1.5)",
            expect_edited_dxf: "",
            expect_reedited_dxf: "",
        });
    }

    #[test]
    fn staged_python_helix_lifecycle_over_real_ipc() {
        run_mesh_lifecycle(&MeshCase {
            create: concat!(
                "doc.create_entity('Helix', axis_base_point=P(0.0, 0.0, 0.0), axis_vector=P(0.0, 0.0, 1.0),\n",
                "    start_point=P(5.0, 0.0, 0.0), radius=5.0, turns=3.0, turn_height=2.0)\n"),
            edit: concat!(
                "helix = doc.entities[HANDLE]\n",
                "with doc.transaction('Edit helix'):\n",
                "    helix.turns = 5.0\n",
                "    helix.handedness = False\n",
                "doc.selection = [helix]\n"),
            rejects: &[
                ("'radius':0.0", "greater than zero"),
                ("'turns':-1.0", "greater than zero"),
                ("'turn_height':float('nan')", "greater than zero"),
                ("'axis_vector':{'x':0.0,'y':0.0,'z':0.0}", "nonzero"),
                ("'start_point':{'x':0.0,'y':0.0,'z':3.0}", "on the axis"),
                ("'spline':{}", "read-only"),
                ("'major_version':1", "read-only"),
            ],
            is_kind: |entity| matches!(entity, EntityType::Helix(_)),
            digest: |entity| match entity {
                EntityType::Helix(h) => format!("r{} t{} h{} endz{:.1} ccw{} cp{}", h.radius, h.turns, h.turn_height,
                    h.spline.control_points.last().map_or(f64::NAN, |p| p.z), h.handedness,
                    !h.spline.control_points.is_empty()),
                _ => "wrong kind".into(),
            },
            reedit: |entity| if let EntityType::Helix(h) = entity { h.turns = 6.0; },
            expect_created: "r5 t3 h2 endz6.0 ccwtrue cptrue",
            expect_edited: "r5 t5 h2 endz10.0 ccwfalse cptrue",
            expect_reedited: "r5 t6 h2 endz10.0 ccwfalse cptrue",
            // BLOCKER: the DXF reader parses boolean group 290 as an i16 and
            // never applies it, so handedness always reopens counter-clockwise.
            expect_edited_dxf: "r5 t5 h2 endz10.0 ccwtrue cptrue",
            expect_reedited_dxf: "r5 t6 h2 endz10.0 ccwtrue cptrue",
        });
    }

    #[test]
    fn staged_python_raster_image_lifecycle_over_real_ipc() {
        image::RgbaImage::from_pixel(8, 4, image::Rgba([200, 30, 30, 255]))
            .save(std::env::temp_dir().join("ocs_raster_test.png"))
            .unwrap();
        run_mesh_lifecycle(&MeshCase {
            create: concat!(
                "doc.create_entity('RasterImage', file_path='TMPDIR/ocs_raster_test.png',\n",
                "    insertion_point=P(10.0, 20.0, 0.0), u_vector=P(0.5, 0.0, 0.0), v_vector=P(0.0, 0.5, 0.0))\n"),
            edit: concat!(
                "img = doc.entities[HANDLE]\n",
                "cb = img.clip_boundary\n",
                "cb['vertices'] = [{'x': 1.0, 'y': 1.0}, {'x': 7.0, 'y': 3.0}]\n",
                "with doc.transaction('Edit image'):\n",
                "    img.insertion_point = (12.0, 22.0, 0.0)\n",
                "    img.u_vector = (1.0, 0.0, 0.0)\n",
                "    img.brightness = 70\n",
                "    img.contrast = 40\n",
                "    img.fade = 10\n",
                "    img.clip_boundary = cb\n",
                "    img.clipping_enabled = True\n",
                "doc.selection = [img]\n"),
            rejects: &[
                ("'file_path':'other.png'", "fixed at creation"),
                ("'brightness':101", "within 0..=100"),
                ("'u_vector':{'x':0.0,'y':0.0,'z':0.0}", "not parallel"),
                ("'v_vector':{'x':2.0,'y':0.0,'z':0.0}", "not parallel"),
                ("'size':{'x':5.0,'y':5.0}", "read-only"),
                ("'definition_handle':5", "read-only"),
                ("'flags':64", "unknown or out-of-range bits"),
                ("'clip_boundary':{'vertices':[]}", "at least 2"),
            ],
            is_kind: |entity| matches!(entity, EntityType::RasterImage(_)),
            digest: |entity| match entity {
                EntityType::RasterImage(i) => format!("{} {:.1},{:.1} u{:.1} b{} c{} f{} clip{} n{} {}x{} def{}",
                    std::path::Path::new(&i.file_path).file_name().unwrap().to_string_lossy(),
                    i.insertion_point.x, i.insertion_point.y, i.u_vector.x, i.brightness, i.contrast, i.fade,
                    i.clipping_enabled, i.clip_boundary.vertices.len(), i.size.x, i.size.y, i.definition_handle.is_some()),
                _ => "wrong kind".into(),
            },
            reedit: |entity| if let EntityType::RasterImage(i) = entity { i.brightness = 80; },
            expect_created: "ocs_raster_test.png 10.0,20.0 u0.5 b50 c50 f0 clipfalse n2 8x4 deftrue",
            expect_edited: "ocs_raster_test.png 12.0,22.0 u1.0 b70 c40 f10 cliptrue n2 8x4 deftrue",
            expect_reedited: "ocs_raster_test.png 12.0,22.0 u1.0 b80 c40 f10 cliptrue n2 8x4 deftrue",
            expect_edited_dxf: "",
            expect_reedited_dxf: "",
        });
    }

    #[test]
    fn staged_python_wipeout_lifecycle_over_real_ipc() {
        run_mesh_lifecycle(&MeshCase {
            create: concat!(
                "doc.create_entity('Wipeout', insertion_point=P(0.0, 0.0, 0.0),\n",
                "    u_vector=P(10.0, 0.0, 0.0), v_vector=P(0.0, 6.0, 0.0))\n"),
            edit: concat!(
                "w = doc.entities[HANDLE]\n",
                "with doc.transaction('Edit wipeout'):\n",
                "    w.clip_type = 'Polygonal'\n",
                "    w.clip_boundary_vertices = [{'x': -0.5, 'y': -0.5}, {'x': 0.5, 'y': -0.5}, {'x': 0.0, 'y': 0.5}]\n",
                "    w.insertion_point = (2.0, 3.0, 0.0)\n",
                "    w.clip_mode = 'Inside'\n",
                "doc.selection = [w]\n"),
            rejects: &[
                ("'clip_boundary_vertices':[{'x':0.0,'y':0.0}]", "at least 3"),
                ("'clip_type':'Rectangular'", "exactly 2"),
                ("'u_vector':{'x':0.0,'y':0.0,'z':0.0}", "not parallel"),
                ("'brightness':200", "within 0..=100"),
                ("'size':{'x':0.0,'y':1.0}", "greater than zero"),
                ("'insertion_point':{'x':float('nan'),'y':0.0,'z':0.0}", "finite"),
                ("'definition_handle':5", "read-only"),
                ("'clip_type':'Circle'", "unsupported"),
            ],
            is_kind: |entity| matches!(entity, EntityType::Wipeout(_)),
            digest: |entity| match entity {
                EntityType::Wipeout(w) => format!("{:?} {:?} n{} at{:.1},{:.1} u{:.1} v{:.1}", w.clip_type, w.clip_mode,
                    w.clip_boundary_vertices.len(), w.insertion_point.x, w.insertion_point.y, w.u_vector.x, w.v_vector.y),
                _ => "wrong kind".into(),
            },
            reedit: |entity| if let EntityType::Wipeout(w) = entity { w.u_vector.x = 12.0; },
            expect_created: "Rectangular Outside n2 at0.0,0.0 u10.0 v6.0",
            expect_edited: "Polygonal Inside n3 at2.0,3.0 u10.0 v6.0",
            expect_reedited: "Polygonal Inside n3 at2.0,3.0 u12.0 v6.0",
            expect_edited_dxf: "",
            expect_reedited_dxf: "",
        });
    }

    #[test]
    fn staged_python_underlay_lifecycle_over_real_ipc() {
        run_mesh_lifecycle(&MeshCase {
            create: concat!(
                "doc.create_entity('Underlay', underlay_type='Pdf', definition_handle=DEFHANDLE,\n",
                "    insertion_point=P(5.0, 5.0, 0.0), x_scale=2.0, y_scale=2.0)\n"),
            edit: concat!(
                "u = doc.entities[HANDLE]\n",
                "with doc.transaction('Edit underlay'):\n",
                "    u.insertion_point = (8.0, 9.0, 0.0)\n",
                "    u.x_scale = 3.0\n",
                "    u.y_scale = 3.0\n",
                "    u.rotation = 0.5\n",
                "    u.contrast = 70\n",
                "    u.fade = 20\n",
                "    u.clip_boundary_vertices = [{'x': 0.0, 'y': 0.0}, {'x': 4.0, 'y': 3.0}]\n",
                "doc.selection = [u]\n"),
            rejects: &[
                ("'definition_handle':999999", "does not exist"),
                ("'underlay_type':'Dwf'", "does not match its definition"),
                ("'x_scale':0.0", "nonzero"),
                ("'rotation':float('nan')", "finite"),
                ("'contrast':101", "within 0..=100"),
                ("'normal':{'x':0.0,'y':0.0,'z':0.0}", "nonzero"),
                ("'clip_boundary_vertices':[{'x':1.0,'y':1.0}]", "at least 2"),
                ("'flags':64", "unknown or out-of-range bits"),
            ],
            is_kind: |entity| matches!(entity, EntityType::Underlay(_)),
            digest: |entity| match entity {
                EntityType::Underlay(u) => format!("{:?} at{:.1},{:.1} s{:.1} r{:.1} c{} f{} clip{}", u.underlay_type,
                    u.insertion_point.x, u.insertion_point.y, u.x_scale, u.rotation, u.contrast, u.fade,
                    u.clip_boundary_vertices.len()),
                _ => "wrong kind".into(),
            },
            reedit: |entity| if let EntityType::Underlay(u) = entity { u.x_scale = 4.0; },
            expect_created: "Pdf at5.0,5.0 s2.0 r0.0 c100 f0 clip0",
            expect_edited: "Pdf at8.0,9.0 s3.0 r0.5 c70 f20 clip2",
            expect_reedited: "Pdf at8.0,9.0 s4.0 r0.5 c70 f20 clip2",
            // BLOCKER: DXF stores the rotation in degrees and the reader returns
            // it unconverted, so 0.5 rad reopens as 28.6 and a second save-reopen compounds it to 1641.4.
            expect_edited_dxf: "Pdf at8.0,9.0 s3.0 r28.6 c70 f20 clip2",
            expect_reedited_dxf: "Pdf at8.0,9.0 s4.0 r1641.4 c70 f20 clip2",
        });
    }

    #[test]
    fn staged_python_viewport_lifecycle_over_real_ipc() {
        run_mesh_lifecycle(&MeshCase {
            create: concat!(
                "doc.create_entity('Viewport', owner_handle=PAPERSPACE, center=P(100.0, 100.0, 0.0),\n",
                "    width=80.0, height=60.0, view_center=P(0.0, 0.0, 0.0), view_height=50.0)\n"),
            edit: concat!(
                "vp = doc.entities[HANDLE]\n",
                "with doc.transaction('Edit viewport'):\n",
                "    vp.center = (120.0, 110.0, 0.0)\n",
                "    vp.width = 90.0\n",
                "    vp.view_height = 75.0\n",
                "    vp.twist_angle = 0.25\n",
                "    vp.frozen_layers = [LAYER0]\n",
                "doc.selection = [vp]\n"),
            rejects: &[
                ("'width':0.0", "greater than zero"),
                ("'view_height':float('nan')", "greater than zero"),
                ("'view_direction':{'x':0.0,'y':0.0,'z':0.0}", "nonzero"),
                ("'frozen_layers':[999999]", "does not exist"),
                ("'circle_sides':2", "at least 3"),
                ("'id':7", "read-only"),
                ("'clip_boundary_handle':5", "read-only"),
            ],
            is_kind: |entity| matches!(entity, EntityType::Viewport(_)),
            digest: |entity| match entity {
                EntityType::Viewport(v) => format!("at{:.1},{:.1} {:.1}x{:.1} vh{:.1} tw{:.2} frozen{}", v.center.x, v.center.y,
                    v.width, v.height, v.view_height, v.twist_angle, v.frozen_layers.len()),
                _ => "wrong kind".into(),
            },
            reedit: |entity| if let EntityType::Viewport(v) = entity { v.width = 95.0; },
            expect_created: "at100.0,100.0 80.0x60.0 vh50.0 tw0.00 frozen0",
            expect_edited: "at120.0,110.0 90.0x60.0 vh75.0 tw0.25 frozen1",
            expect_reedited: "at120.0,110.0 95.0x60.0 vh75.0 tw0.25 frozen1",
            expect_edited_dxf: "",
            expect_reedited_dxf: "",
        });
    }

    #[test]
    fn staged_python_point_pick_and_cancel_over_real_ipc() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let process = {
            let mut host = HostSession::new(&mut app, 0);
            std::sync::Arc::new(
                ocs_plugin_api::process::PluginProcess::spawn(
                    std::path::Path::new(&plugin_path),
                    &mut host,
                crate::plugin::v4_support::notification_handler(),
                )
                .expect("spawn staged Python plugin"),
            )
        };
        let request = |app: &mut OpenCADStudio, prompt: &str, entity: bool| {
            let mut started = None;
            {
                let mut host = HostSession::new(app, 0);
                let method = if entity {
                    "request_entity"
                } else {
                    "request_point"
                };
                let cmd = format!("PY_EVAL ocs.active_document.{method}('{prompt}')");
                assert!(process
                    .dispatch(&mut host, &cmd, &mut |id| started = Some(id))
                    .unwrap());
            }
            let output = app.command_line.history.last().unwrap().text.clone();
            let token = output
                .split_whitespace()
                .find_map(|part| part.parse::<u64>().ok())
                .unwrap_or_else(|| panic!("Python pick token missing from {output:?}"));
            let id = started.expect("Python request started an interactive command");
            app.set_active_command(
                0,
                Box::new(PluginProcessInteractiveAdapter::new(
                    std::sync::Arc::clone(&process),
                    id,
                )),
            );
            token
        };

        let picked_token = request(&mut app, "Pick a point", false);
        let result = app.tabs[0]
            .active_cmd
            .as_mut()
            .unwrap()
            .on_point(glam::DVec3::new(3.0, 4.0, 0.0));
        let _ = app.apply_cmd_result(result);
        assert!(app.tabs[0].active_cmd.is_none());
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process
                .dispatch(
                    &mut host,
                    &format!("PY_EVAL ocs.active_document.poll_input({picked_token})"),
                    &mut |_| {}
                )
                .unwrap());
        }
        assert!(app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("point"));

        let target = {
            let mut host = HostSession::new(&mut app, 0);
            host.add_entity(EntityType::Line(Line::from_points(
                acadrust::types::Vector3::new(6.0, 7.0, 0.0),
                acadrust::types::Vector3::new(9.0, 7.0, 0.0),
            )))
        };
        let entity_token = request(&mut app, "Pick an entity", true);
        assert!(app.tabs[0].active_cmd.as_ref().unwrap().needs_entity_pick());
        let result = app.tabs[0]
            .active_cmd
            .as_mut()
            .unwrap()
            .on_entity_pick(target, glam::DVec3::new(6.0, 7.0, 0.0));
        let _ = app.apply_cmd_result(result);
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process
                .dispatch(
                    &mut host,
                    &format!("PY_EVAL ocs.active_document.poll_input({entity_token})"),
                    &mut |_| {}
                )
                .unwrap());
        }
        assert!(app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("entity"));
        assert!(app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains(&target.value().to_string()));

        let cancelled_token = request(&mut app, "Cancel a point", false);
        let result = app.tabs[0].active_cmd.as_mut().unwrap().on_enter();
        let _ = app.apply_cmd_result(result);
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process
                .dispatch(
                    &mut host,
                    &format!("PY_EVAL ocs.active_document.poll_input({cancelled_token})"),
                    &mut |_| {}
                )
                .unwrap());
        }
        assert!(app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("cancelled"));
    }

    #[test]
    fn staged_python_tabs_isolate_tokens_and_notifications() {
        let Some(plugin_path) = std::env::var_os("OCS_TEST_PYTHON_PLUGIN") else {
            return;
        };
        let runner_path = std::env::var_os("OCS_PLUGIN_RUNNER_EXE")
            .expect("set OCS_PLUGIN_RUNNER_EXE to the built OpenCADStudio executable");
        assert!(std::path::Path::new(&runner_path).is_file());
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        app.tabs
            .push(crate::app::document::DocumentTab::new_drawing(2));
        app.tabs[1].is_start = false;
        let tab0 = app.tabs[0].id;
        let tab1 = app.tabs[1].id;
        assert_ne!(tab0, tab1);
        let process = {
            let mut host = HostSession::new(&mut app, 0);
            std::sync::Arc::new(
                ocs_plugin_api::process::PluginProcess::spawn(
                    std::path::Path::new(&plugin_path),
                    &mut host,
                crate::plugin::v4_support::notification_handler(),
                )
                .expect("spawn staged Python plugin"),
            )
        };
        let mut started = None;
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process
                .dispatch(
                    &mut host,
                    "PY_EVAL ocs.active_document.request_point('Tab zero')",
                    &mut |id| started = Some(id)
                )
                .unwrap());
        }
        let token = app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .split_whitespace()
            .find_map(|part| part.parse::<u64>().ok())
            .expect("pick token");
        app.set_active_command(
            0,
            Box::new(PluginProcessInteractiveAdapter::new(
                std::sync::Arc::clone(&process),
                started.unwrap(),
            )),
        );
        {
            let mut host = HostSession::new(&mut app, 1);
            assert!(process
                .dispatch(
                    &mut host,
                    &format!("PY_EVAL ocs.active_document.poll_input({token})"),
                    &mut |_| {}
                )
                .unwrap());
        }
        assert!(
            app.command_line
                .history
                .last()
                .unwrap()
                .text
                .contains("None"),
            "other tab must not consume the pending token"
        );
        let result = app.tabs[0]
            .active_cmd
            .as_mut()
            .unwrap()
            .on_point(glam::DVec3::new(2.0, 3.0, 0.0));
        let _ = app.apply_cmd_result(result);
        {
            let mut host = HostSession::new(&mut app, 1);
            assert!(process
                .dispatch(
                    &mut host,
                    &format!("PY_EVAL ocs.active_document.poll_input({token})"),
                    &mut |_| {}
                )
                .unwrap());
        }
        assert!(app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("None"));
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process
                .dispatch(
                    &mut host,
                    &format!("PY_EVAL ocs.active_document.poll_input({token})"),
                    &mut |_| {}
                )
                .unwrap());
        }
        assert!(app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .contains("point"));

        use ocs_plugin_api::host::HostNotification;
        process
            .notify_plugin(
                None,
                HostNotification::DrawingChanged {
                    tab_id: tab0,
                    epoch: 101,
                },
            )
            .unwrap();
        process
            .notify_plugin(
                None,
                HostNotification::DrawingChanged {
                    tab_id: tab1,
                    epoch: 202,
                },
            )
            .unwrap();
        // Notifications are best-effort and consumed by the runner's reader
        // thread before its next Dispatch. Give that thread a bounded handoff.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let poll_events_until = |app: &mut OpenCADStudio, tab: usize, epoch: &str| {
            let mut output = String::new();
            for _ in 0..10 {
                {
                    let mut host = HostSession::new(app, tab);
                    assert!(process
                        .dispatch(
                            &mut host,
                            "PY_EVAL ocs.active_document.poll_events()",
                            &mut |_| {}
                        )
                        .unwrap());
                }
                output = app.command_line.history.last().unwrap().text.clone();
                if output.contains(epoch) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            output
        };
        let other_events = poll_events_until(&mut app, 1, "202");
        assert!(
            other_events.contains("202") && !other_events.contains("101"),
            "{other_events}"
        );
        let first_events = poll_events_until(&mut app, 0, "101");
        assert!(
            first_events.contains("101") && !first_events.contains("202"),
            "{first_events}"
        );

        for epoch in 0..257 {
            process
                .notify_plugin(
                    None,
                    HostNotification::DrawingChanged {
                        tab_id: tab0,
                        epoch,
                    },
                )
                .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process
                .dispatch(
                    &mut host,
                    "PY_EVAL ocs.active_document.poll_events()",
                    &mut |_| {}
                )
                .unwrap());
        }
        let overflow = &app.command_line.history.last().unwrap().text;
        assert!(
            overflow.contains("overflow") && overflow.contains("dropped"),
            "{overflow}"
        );

        process
            .notify_plugin(
                None,
                HostNotification::DrawingChanged {
                    tab_id: tab0,
                    epoch: 303,
                },
            )
            .unwrap();
        process
            .notify_plugin(None, HostNotification::DocumentTabClosed { tab_id: tab0 })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        {
            let mut host = HostSession::new(&mut app, 0);
            assert!(process
                .dispatch(
                    &mut host,
                    "PY_EVAL ocs.active_document.poll_events()",
                    &mut |_| {}
                )
                .unwrap());
        }
        assert!(
            app.command_line.history.last().unwrap().text.contains("[]"),
            "closing a tab must discard its queued events"
        );
    }

    #[test]
    fn unmapped_canvas_kind_layer_change_undoes() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let handle;
        {
            let mut host = HostSession::new(&mut app, 0);
            handle = host.add_entity(EntityType::MLine(acadrust::entities::MLine::default()));
            let original = host.document().get_entity(handle).unwrap();
            let changed =
                ocs_plugin_api::entity_coverage::patch_canvas_layer(original, "HATCHES").unwrap();
            host.update_entities_transaction("Move hatch layer", vec![changed])
                .unwrap();
            assert_eq!(
                host.document().get_entity(handle).unwrap().common().layer,
                "HATCHES"
            );
        }
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 1);
        app.undo_steps(1);
        assert_eq!(
            app.tabs[0]
                .scene
                .document
                .get_entity(handle)
                .unwrap()
                .common()
                .layer,
            "0"
        );
    }

    #[test]
    fn insert_transform_transaction_undoes_without_changing_block_identity() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let handle;
        {
            let mut host = HostSession::new(&mut app, 0);
            let insert = acadrust::entities::Insert::new(
                "DOOR",
                acadrust::types::Vector3::new(1.0, 2.0, 0.0),
            );
            handle = host.add_entity(EntityType::Insert(insert));
            let mut changed = host.document().get_entity(handle).unwrap().clone();
            if let EntityType::Insert(insert) = &mut changed {
                insert.insert_point.x = 4.0;
                insert.set_x_scale(2.0);
            }
            host.update_entities_transaction("Move block", vec![changed])
                .unwrap();
            assert!(
                matches!(host.document().get_entity(handle), Some(EntityType::Insert(insert))
                if insert.insert_point.x == 4.0 && insert.x_scale() == 2.0 && insert.block_name == "DOOR")
            );
        }
        app.finish_pending_history(0);
        assert_eq!(app.tabs[0].history.undo_stack.len(), 1);
        app.undo_steps(1);
        assert!(
            matches!(app.tabs[0].scene.document.get_entity(handle), Some(EntityType::Insert(insert))
            if insert.insert_point.x == 1.0 && insert.x_scale() == 1.0 && insert.block_name == "DOOR")
        );
    }

    #[test]
    fn system_variables_change_without_command_reentry() {
        let mut app = OpenCADStudio::new_for_test();
        app.tabs[0].is_start = false;
        let mut host = HostSession::new(&mut app, 0);
        assert_eq!(
            host.system_variable("clayer"),
            Some(HostSettingValue::Text("0".into()))
        );
        assert_eq!(
            host.system_variable("SNAPANG"),
            Some(HostSettingValue::Number(0.0))
        );
        assert!(host
            .set_system_variable("CLAYER", HostSettingValue::Text("Missing".into()))
            .is_err());
        assert_eq!(
            host.system_variable("CLAYER"),
            Some(HostSettingValue::Text("0".into()))
        );

        let mut point = Point::new();
        point.common.layer = "Annotations".into();
        host.add_entity(EntityType::Point(point));
        assert_eq!(
            host.set_system_variable("clayer", HostSettingValue::Text("Annotations".into())),
            Ok(HostSettingValue::Text("Annotations".into()))
        );
        assert_eq!(
            host.system_variable("CLAYER"),
            Some(HostSettingValue::Text("Annotations".into()))
        );
        assert_eq!(host.app.tabs[0].active_layer, "Annotations");
        assert_eq!(host.app.tabs[0].layers.current_layer, "Annotations");
        assert_eq!(host.app.ribbon.active_layer, "Annotations");
        assert!(host.app.tabs[0].dirty);

        assert_eq!(
            host.set_system_variable("snapang", HostSettingValue::Number(450.0)),
            Ok(HostSettingValue::Number(90.0))
        );
        assert_eq!(
            host.system_variable("SNAPANG"),
            Some(HostSettingValue::Number(90.0))
        );
        assert!(host
            .set_system_variable("SNAPANG", HostSettingValue::Number(f64::INFINITY))
            .is_err());
        assert!(host
            .set_system_variable("SNAPANG", HostSettingValue::Number(f64::MAX))
            .is_err());
        assert_eq!(
            host.system_variable("SNAPANG"),
            Some(HostSettingValue::Number(90.0))
        );
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
        let got = host
            .document()
            .get_entity(h)
            .expect("entity kept its handle");
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
        for pt in [
            glam::DVec3::new(0.0, 0.0, 0.0),
            glam::DVec3::new(5.0, 5.0, 0.0),
        ] {
            let r = app.tabs[0].active_cmd.as_mut().unwrap().on_point(pt);
            let _ = app.apply_cmd_result(r);
        }
        assert_eq!(app.tabs[0].scene.document.entities().count(), 1);
        assert!(
            app.tabs[0].active_cmd.is_none(),
            "command should have ended"
        );
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
        let reader = ocs_plugin_api::shm::SharedDocumentReader::<
            ocs_plugin_api::shm::DocumentViewData,
        >::open(std::path::Path::new(&info.path))
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
        let reader = ocs_plugin_api::shm::SharedDocumentReader::<
            ocs_plugin_api::shm::DocumentViewData,
        >::open(std::path::Path::new(&info.path))
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
