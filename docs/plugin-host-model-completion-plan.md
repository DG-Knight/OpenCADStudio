# OCS canvas entity completion plan

## Goal and current state

Build a general Python document model over the OCS host API for all **43 canvas kinds**. A kind is **complete** only after the per-kind gate below passes. Merely appearing in a converter or allowing a `layer` edit is partial coverage. Keep a property-level record of read/write, read-only, snapshot-only, and unsupported fields in the [43-kind coverage ledger](plugin-host-model-coverage-ledger.md). Do not resume the separate Lisp-to-Python conversion work.

As of 20 September 2026, the working branch is `plugin/host-model-api` at
`a8dd3640` plus the Leader increment, published as draft PR
[#1391](https://github.com/HakanSeven12/OpenCADStudio/pull/1391). Host API is
v7. The Python feature has explicit writes for **31 of 43** canvas kinds: 30
are creatable and `Insert` remains update-only. This is **not** 30 completed
kinds: Tolerance, Shape, AttributeEntity, Hatch, MLine, Dimension, MultiLeader, Table, PolygonMesh, PolyfaceMesh and Mesh pass every completion gate.
AttributeDefinition passes its host and real IPC lifecycle but remains short of
`Complete` because the previously pinned CAD codec dropped optional ATTDEF
fields during DXF reads; revalidate that blocker against the current
`acadrust` revision `7ea4247`. Leader is mapped and integration-tested but blocked at W by three acadrust persistence gaps (see the ledger). Leader and Helix are mapped and integration-tested but blocked at W by acadrust persistence gaps (see the ledger). The other 12 canvas kinds have only `layer`
writes through the document model. All 43 have raw typed snapshots. There are
three internal records and two opaque fallbacks outside the 43 canvas kinds.

Repositories:

- OCS host: `/Users/felix/Documents/MacApps/OpenCADStudio`
- Bundled Python adapter: `/Users/felix/Documents/MacApps/OpenCADStudio/plugins/opencad-python`
- Host coverage policy: `crates/ocs_plugin_api/entity_coverage_policy.json`
- Host coverage/validation: `crates/ocs_plugin_api/src/entity_coverage.rs` and `build.rs`
- Adapter mapping: `plugins/opencad-python/entity_manifest.json`,
  `plugins/opencad-python/build/generate.rs`,
  `plugins/opencad-python/src/ocs_module.rs`, and
  `plugins/opencad-python/src/document_model.py`
- Host design notes: `docs/plugin-host-model.md`

Preserve unrelated untracked files. Host and adapter changes now live in the
same OCS repository and commit. Push the feature branch only to the
`felixriestra/OpenCADStudio` fork; PR #1391 updates automatically. Never push a
feature commit directly to H7's `main`. Verify both upstream `main` and the fork
branch before each push.

## First: complete the originally agreed Phase 8

The earlier Phase 8 was **integration and compatibility**, not a new entity kind. `Insert` was added afterward as a separate coverage increment. Before marking any kind complete, establish an executable integration harness that exercises the actual plugin runner and IPC, not only direct Rust conversion helpers:

1. Stage the in-tree v7 plugin with `bash plugins/opencad-python/tools/stage-bundled.sh <stage directory> [--debug]`. Verify the staged `plugin.toml` says API v7 and its acadrust source and Rust compiler match OCS. Keep the repository-relative `ocs_plugin_api` path; never check in a machine-local path.
2. Through real IPC, create, inspect, edit, and delete a representative entity. Verify an invalid multi-entity transaction changes nothing, a valid transaction produces one undo entry, and undo **and redo** restore the expected geometry and references.
3. In the running GUI, exercise a real point pick, entity pick, cancellation/Escape, and plugin command cancellation. Verify drawing/selection/command notifications, including overflow recovery.
4. Repeat an input request and event poll across two tabs; verify token and notification isolation and cleanup when a tab closes.
5. Save and reopen representative edits as **DWG and DXF**, verifying the actual geometry and preserved unmapped fields. Use a real viewport/canvas check where the entity has visible geometry; a converter round trip alone does not satisfy this.
6. Record platform, OCS/acadrust revisions, fixture files, commands, results, and any engine limitation. Keep repeatable tests in the relevant repositories.

Phase 8 is complete only when these checks pass through the bundled plugin and
real runner boundary. If a test is blocked by OCS or acadrust behavior, record
the specific failure and leave Phase 8 open.

## Per-kind completion gate

Use one checklist row per kind in a coverage ledger. Status values: `unstarted`, `mapped`, `integration-tested`, `complete`, or `blocked`; record the blocking dependency. **Do not use “complete” for read-only, update-only, or layer-only support.** For every kind:

1. **Declare coverage:** Every traced field has an accurate catalog status and validation rule. Distinguish ordinary properties from handles, owners, linked records, external assets, and opaque payloads. Snapshot-only fields remain explicitly unmapped. Assert adapter keys match the host catalog.
2. **Create:** A Python script creates a valid entity through the document model in a real document with required tables, styles, definitions, files, and references. Missing or invalid required inputs fail before mutation. Add document-model creation/deletion methods if needed; current `doc.entities` only indexes and iterates, while low-level `ocs.add`/remove APIs exist.
3. **Read and edit:** After creation, read the entity by handle through `doc.entities`; edit at least one defining geometry or placement field and one other meaningful writable field inside `doc.transaction`. Verify that all unmentioned fields, handles, ownership, attached records, and XDATA remain unchanged. Check invalid values and forbidden reference edits fail atomically.
4. **Delete and history:** Delete the created entity through the Python interface. Prove create, edit, and delete have defined undo/redo behavior in the host, including grouped writes and failure rollback. Record any transaction API extension and bump the host API version if its ABI changes; keep older plugin compatibility.
5. **Real IPC and canvas:** Run the above through the staged plugin and actual OCS IPC. Verify visibility/geometry in the canvas and selection or picking in the GUI. For nonvisual entities, verify their observable document or layout behavior and explain the oracle.
6. **Persistence:** Save/reopen DWG **and** DXF fixtures. Compare kind, geometry, important metadata, handles/references, and preserved fields after reopen; edit again after reopen. Test at least one valid edge case for that kind. Do not count a JSON or in-memory round trip as file persistence.
7. **Packaged build:** Run the in-tree adapter tests with `--locked`, stage the
   plugin with `stage-bundled.sh`, and load it from the macOS application
   resources through the separate runner process. The staged manifest must
   match the host's Rust compiler and acadrust source.

If OCS/acadrust cannot create or serialize a kind reliably, mark it **blocked**, add a regression test or fixture demonstrating the failure, and move to the next independent kind. An explicit unsupported status is honest progress, not completion.

## Existing mapped kinds: audit before claiming completion

Audit these against the same gate: `Point`, `Line`, `Circle`, `Arc`, `Ellipse`, `Polyline`, `Polyline2D`, `Polyline3D`, `LwPolyline`, `Spline`, `Text`, `MText`, `Ray`, `XLine`, `Solid`, `Face3D`, `Insert`. `Insert` currently has transform edits but **no Python creation**; its block name is read-only and attached attributes are snapshot-only. Earlier tests include some direct DWG round trips, but there is no evidence that every mapped kind passes real IPC, GUI, deletion, undo/redo, and both file formats. Mark each independently after testing.

## Ordered 26-kind expansion queue

The order below groups shared implementation work. Complete a kind only through the gate above. Within a group, move past a blocked kind after recording the exact dependency. “Minimum exercise” identifies a real entity-specific create/edit/delete case; it does not make other fields automatically writable.

| # | Kind | Minimum entity-specific exercise and dependency |
|---:|---|---|
| 1 | Tolerance | Create a feature-control frame with text/style; move and edit its text/direction; check style reference and display. |
| 2 | Shape | Create with an installed SHX shape/style; change insertion, size, rotation; verify the named glyph and style survive reopen. |
| 3 | AttributeDefinition | Create in a block definition; edit tag, default text, and placement; verify the block’s definition and linked inserts. |
| 4 | AttributeEntity | Create as a block reference attribute; edit value and placement; verify parent `Insert`, handles, and sequence linkage. |
| 5 | Hatch | Create a closed-boundary solid hatch and a patterned hatch; edit boundary/pattern; verify fill and boundary validity. |
| 6 | Leader | Create with vertices and an annotation; edit path and annotation placement; preserve annotation handle. |
| 7 | MLine | Create against a valid MLine style; edit vertices and scale; verify style element offsets and joins. |
| 8 | Dimension | Create at least one supported dimension subtype with style and reference points; edit defining geometry and displayed measurement. Track subtype coverage separately. |
| 9 | MultiLeader | Create text and leader paths using a valid style; edit landing/vertices and text; preserve nested context and annotation links. |
| 10 | Table | Create rows, columns, style, and cells; edit size and one cell value; preserve merged ranges and block/style links. |
| 11 | PolygonMesh | Create a grid with defined M/N counts and vertices; edit a vertex and density; reject inconsistent counts. |
| 12 | PolyfaceMesh | Create vertices/faces; edit a vertex and face indices; reject invalid indices and preserve sequence records. |
| 13 | Mesh | Create vertices/faces/edges; edit one vertex and topology; reject out-of-range indices and verify rendered faces. |
| 14 | Helix | Create with axis, radius, turns, and height; edit a defining parameter and verify the derived curve plus embedded spline data. |
| 15 | RasterImage | Create with a resolvable image definition/file; edit placement, vectors, and clipping; verify display and definition handles. |
| 16 | Wipeout | Create with a valid clip polygon; edit boundary and placement; verify masking, clip mode, and image-definition links. |
| 17 | Underlay | Create with a valid PDF/DWF/DGN definition; edit transform and clipping; verify external resource and definition handle. |
| 18 | Viewport | Create in the appropriate layout; edit center/size/view target; verify visible viewport and frozen-layer/clip references. |
| 19 | ViewBorder | Create with its required view/scale references; edit border geometry; verify linked viewport/view records. |
| 20 | SectionSymbol | Create with view style and point records; edit endpoints/label; preserve view-representation links. |
| 21 | Light | Create a light with position/target; edit intensity and direction; verify color/photometric state and visible glyph. |
| 22 | Region | Create/import a valid region payload; transform/edit supported geometry, then verify ACIS payload after both file formats. |
| 23 | Body | Create/import a valid body payload; apply a supported transform/edit; verify ACIS and history/reference preservation. |
| 24 | Solid3D | Create/import a valid solid; edit a supported geometric operation; verify ACIS, topology, and history after reopen. |
| 25 | Surface | Create/import a valid surface; edit a supported parameter/transform; verify surface payload, isolines, and history. |
| 26 | Ole2Frame | Create with a valid embedded object payload; edit frame geometry; verify storage, aspect behavior, and persistence. |

The last five may require new engine capabilities rather than only Python converters. Do not deserialize and rewrite their opaque payloads unless lossless behavior is proven. If a listed kind has no meaningful safe Python creation/edit path, keep it marked blocked or partial and report why; do not relabel it complete to reach 43/43.

## Work pattern for each coding increment

1. Inspect the acadrust entity struct, constructors, private fields, serializer, drawing path, and linked tables/objects. Record the intended editable/read-only/unmapped properties first.
2. Put generic validation and transaction behavior in the OCS fork. Keep the Python adapter thin; add a manifest override only when the generator cannot safely express a field.
3. Add focused host validation, adapter conversion, document-model, real transaction/undo/redo, IPC, GUI, and DWG/DXF fixture checks. Record test evidence in the ledger, including which checks were manual.
4. Run `cargo test -p ocs_plugin_api --features host --lib` and the relevant
   app host tests. Run
   `cargo test --locked --features experimental-host-model --manifest-path plugins/opencad-python/Cargo.toml`
   plus
   `python3 -m unittest discover -s plugins/opencad-python/tests -v`.
5. Check `git diff --check` and repository status. Stage only task files,
   preserve unrelated untracked files, commit host and adapter changes
   together, push `plugin/host-model-api` to the Felix fork, and record the
   resulting commit and next unfinished gate in the ledger.

## Handoff start point

Continue with **RasterImage** (queue item 10). Leader is done except for its W
gate; when the engine writes DXF group 340, reads group 213 and the DWG
R2010+ text-size question is decided, flip the canary assertions in
`staged_python_leader_lifecycle_over_real_ipc` and mark it `Complete`.
AttributeDefinition remains mapped and integration-tested but open at W; its
real IPC lifecycle now passes against acadrust `7ea4247`, so audit the full
optional-property DXF round trip and close or retain the blocker. Keep the
exact queue above and update statuses with evidence after each increment.

## Phase 8 progress log

- Added a V4 local-socket nested-request test for `UpdateEntitiesTransaction` using a simulated runner peer and a recording host. This proves the request and response cross the V4 IPC framing and reach `HostApi`; it does **not** prove the actual Python plugin runs through IPC.
- Extended the OCS app-host Point batch test to assert redo after undo. This proves the real app history path for those edits; creation/deletion history remains untested by this case.
- Staged the actual debug Python cdylib with `stage-bundled.sh`. Its generated
  manifest declares API v7 and the exact OCS acadrust and Rust compiler
  fingerprints. The plugin now builds from `plugins/opencad-python` in this
  repository and ships from the macOS application resources.
- The opt-in `staged_python_plugin_line_lifecycle_over_real_ipc` app test passed with the staged v7 Python plugin and built OCS executable. It creates, edits, and deletes a Line through Python over the actual runner IPC; confirms three undo entries and undo/redo; and writes/reopens the edited Line as DWG and DXF. It does not exercise GUI picking, multiple tabs, or cancellation. Re-run with `OCS_TEST_PYTHON_PLUGIN=<staged dylib> OCS_PLUGIN_RUNNER_EXE=<built OpenCADStudio executable> cargo test app::plugin_host::tests::staged_python_plugin_line_lifecycle_over_real_ipc --lib -- --test-threads=1 --nocapture` from the OCS checkout.
- The same Line test now also sends a duplicate-handle batch through Python and confirms the host rejects it without changing geometry or creating an undo entry.
- The opt-in `staged_python_point_pick_and_cancel_over_real_ipc` test passes with the same runner. It requests point and entity picks through Python, drives the app's active-command point/object-pick callbacks, polls the returned tokens through a later Python run, and verifies Enter cancellation. The callbacks are invoked by the test harness, not by a physical GUI pointer event; live GUI verification remains open.
- The opt-in `staged_python_tabs_isolate_tokens_and_notifications` test passes with two real OCS document tabs and the staged runner. A token created in tab 0 cannot be observed or consumed in tab 1 before or after completion. Host-to-plugin DrawingChanged notifications for both tab ids are delivered over IPC and drained only by their owning tab. A 257-event burst reports overflow, and `DocumentTabClosed` discards queued events for that tab.
- Added `doc.create_entity(kind, **properties)` and `doc.delete_entity(entity_or_handle)` to the high-level Python document model. Unit tests cover forwarding and managed identity fields. The real staged-plugin Line lifecycle now uses these methods rather than low-level `ocs.add`/`remove_entity`.
- Closed the first new-kind increment for `Tolerance`. The host coverage catalog now maps insertion point, direction, normal, text, dimension style name, text height, and gap; validates geometry and document style references; keeps the style handle read-only; and leaves the undocumented DWG short unmapped. The thin adapter adds generated conversion plus stale style-handle clearing when the style name changes.
- `staged_python_tolerance_lifecycle_over_real_ipc` passes the full C/R/E/D/U/I/W/V/P gate with the actual runner: high-level create/read/transaction edit/delete, live canvas selection, undo/redo, invalid direction and missing style rollback, untouched-field preservation, and DWG/DXF reopen. Tolerance is recorded as **Complete** in the ledger. The queue now continues with `Shape`.
- Closed `Shape` with a generated SHX fixture and a real shape-file `TextStyle`. Host-owned binding resolves the Python style name to the stable table handle required by DWG; validation rejects missing/non-shape styles and invalid geometry. The adapter maps all stable shape fields and keeps the bound style handle read-only.
- `staged_python_shape_lifecycle_over_real_ipc` passes C/R/E/D/U/I/W/V/P through the actual runner, checks the real SHX linework before and after DWG/DXF reopen, and proves that invalid size/style edits remain atomic. The DXF renderer fallback now searches named `is_shape_file` styles as well as legacy unnamed styles. Shape is **Complete**; the queue now continues with `AttributeDefinition`.
- Added creation-time `owner_handle` to the generated document-model adapter. It is present in snapshots and accepted by `doc.create_entity`, but rejected after creation; host transactions independently reject owner changes. Generic host validation now requires a non-null canvas owner to resolve to a block record, except `AttributeEntity`, whose owner may be an `Insert`.
- Mapped stable AttributeDefinition fields and validated tag, placement, normal, height, width, angles, text style, line count and block ownership. `embedded_mtext` stays snapshot-readable but unmapped so partial Python edits preserve its linked R2018+ layout payload.
- `staged_python_attribute_definition_lifecycle_over_real_ipc` passes the real create/read/edit/delete/undo/selection/render/validation/packaged-build path using a structurally valid block and linked Insert. Full DWG state and core DXF geometry survive reopen; deletion stays absent after both formats. OCS also stopped double-converting ATTDEF/ATTRIB DXF rotation already converted by acadrust.
- AttributeDefinition remains **Integration-tested / DXF blocked**, not Complete: acadrust `568a12c`'s ATTDEF DXF reader did not restore optional AcDbText fields including width factor. Revalidate this recorded failure against the currently merged `7ea4247` revision before keeping or clearing the W-gate blocker.
- Closed `AttributeEntity` without flattening canonical drawing storage. V4 views and IPC snapshots expose each nested child by handle, while host create/update/delete and transactions rewrite the owning Insert so rendering, serialization and undo remain consistent.
- `staged_python_attribute_entity_lifecycle_over_real_ipc` passes C/R/E/D/U/I/W/V/P with the actual v7 adapter. It creates a linked attribute from its block's ATTDEF, reads and edits value/placement/rotation, selects the parent Insert, renders text, rejects invalid tag/height/owner edits atomically, reopens edited and deleted state in DWG and DXF, and proves three-step undo/redo. AttributeEntity is **Complete**; the queue now continues with `Hatch`.
- Completed `Hatch` with full solid/pattern definitions and typed nested boundary paths. The adapter generator now handles single-payload tagged enums as `{kind, value}` dictionaries, which covers every Hatch edge variant and is reusable by later nested entity models. Unknown adapter properties are rejected instead of being silently ignored.
- `staged_python_hatch_lifecycle_over_real_ipc` passes C/R/E/D/U/I/W/V/P with closed line-loop fixtures: solid creation, conversion to a stored-line pattern, boundary replacement, selection and live render-cache checks, atomic invalid scale/association/open-loop/unmapped-payload rejection, edited/deleted DWG and DXF reopen, post-reopen edits, and three-step undo/redo. Hatch is **Complete**; the queue now continues with `Leader`.
- Mapped `Leader` (host validation and reference binding in `entity_coverage.rs`, policy/build.rs/manifest entries, adapter count 23). `staged_python_leader_lifecycle_over_real_ipc` passes C/R/E/D/U/I/V/P through the real runner. W is blocked by acadrust: the DXF writer omits the annotation handle (340), DXF reads lose `annotation_offset` (213), and DWG R2010+ does not store text size or hookline flag. Each gap is pinned by a canary assertion so it fails loudly once fixed. The queue now continues with `MLine`.
- Completed `MLine`. Host validation and style binding live in `ocs_plugin_api`; derived vertex geometry and element parameters are recomputed by the host (`normalize_scripted_mline`) on scripted create/edit, so scripts supply positions only. The adapter maps `flags` as an integer through a manifest override because the generator cannot classify acadrust's serde-newtype `MLineFlags`. `staged_python_mline_lifecycle_over_real_ipc` passes C/R/E/D/U/I/W/V/P with DWG and DXF reopen. The queue now continues with `Dimension`.
- Completed `Dimension` for all nine subtypes. Because the payload is an enum, the adapter generator gained a `manual_kinds` hook (hand-written `<kind>_to_dict/_from_dict/_apply`) and the host catalog a `synthetic` policy section. The host derives `actual_measurement` and the base definition point on every scripted create/edit. Python ints are now accepted wherever a coordinate or angle is expected. `staged_python_dimension_lifecycle_over_real_ipc` records each subtype separately and all nine pass C/R/E/D/U/I/W/V/P. The queue now continues with `MultiLeader`.
- Completed `MultiLeader`. The adapter generator now converts `Color`, `LineWeight`, mixed unit/payload enums, fixed float arrays and bitflags-2 newtypes, and a `keep_types` override lets a hand-written setter reuse generated converters (used for the merge-style `context` patch). `Leader.override_color` became writable through the same `Color` form; neither file format stores it. `staged_python_multileader_lifecycle_over_real_ipc` passes the full gate with DWG and DXF reopen. The queue now continues with `Table`.
- Completed `Table`. The generator now handles tuples and `usize`; the host keeps DXF cell merge dimensions in step with `merged_ranges`. `staged_python_table_lifecycle_over_real_ipc` passes the full gate with DWG and DXF reopen. The queue now continues with `PolygonMesh`.
- Completed `PolygonMesh`, `PolyfaceMesh` and `Mesh` (queue items 6-8) with one shared `run_mesh_lifecycle` test driver and per-kind host validation. The generator now omits a sub-record's own `EntityCommon` from the scripted form. All three pass the full gate including DWG and DXF reopen with re-edit. The queue now continues with `Helix`.
- Mapped `Helix` with a host-rebuilt spline. `run_mesh_lifecycle` gained per-format DXF expectations so an engine gap is pinned by a canary instead of hidden. All gates pass except W for `handedness` in DXF (reader ignores boolean group 290). The queue now continues with `RasterImage`.
