# OCS canvas kind coverage ledger

Use with [the completion plan](plugin-host-model-completion-plan.md). This ledger is current as of 20 September 2026: **22 of 43 canvas kinds have explicit geometry writes** (21 creatable, plus update-only `Insert`), **4 kinds are complete**, and 21 remain layer-only. `Mapped` means at least one non-layer field is exposed for Python writes; it does **not** mean the kind passes the completion gate. `Update-only` means Python creation is unavailable. `Layer-only` means raw inspection and canvas layer edits exist, but geometry is not mapped. Record actual evidence or a blocking issue before changing any status to `Complete`.

Completion gate codes: **C** = create through Python document model; **R** = read; **E** = edit defining geometry and another property; **D** = delete; **U** = undo/redo and atomic failure; **I** = real IPC plus GUI/canvas pick or observable behavior; **W** = DWG and DXF save/reopen; **V** = invalid-input checks and untouched-field/reference preservation; **P** = build and load the in-tree bundled plugin with a manifest matching the host toolchain and acadrust source. A kind is `Complete` only when **C/R/E/D/U/I/W/V/P** all pass and its property catalog matches implementation. `Blocked` requires a specific dependency or failing fixture.

| Kind | Baseline | Gate evidence / blocker |
|---|---|---|
| Point | Mapped | Host transaction test now checks invalid batch rollback, grouped edit undo, and redo; creation/deletion history, real plugin execution, GUI, and DWG/DXF gates remain pending. |
| Line | Integration-tested | High-level document-model create/delete and transaction edit pass through the actual staged-plugin IPC runner; three undo entries, undo/redo, duplicate-handle and nonfinite-coordinate rejection, full typed-entity preservation, Line object-pick routing, and edited DWG/DXF reopen pass. A second editable property outside defining endpoint geometry still needs integration coverage before this row is complete. |
| Circle | Mapped | Full gate pending. |
| Arc | Mapped | Full gate pending. |
| Ellipse | Mapped | Existing focused DWG conversion test; full gate pending. |
| Polyline | Mapped | Full gate pending. |
| Polyline2D | Mapped | Full gate pending. |
| Polyline3D | Mapped | Full gate pending. |
| LwPolyline | Mapped | Full gate pending. |
| Spline | Mapped | Full gate pending. |
| Text | Mapped | Full gate pending. |
| MText | Mapped | Full gate pending. |
| Ray | Mapped | Focused DWG round trip; full gate pending. |
| XLine | Mapped | Focused DWG round trip; full gate pending. |
| Solid | Mapped | Focused DWG round trip; full gate pending. |
| Face3D | Mapped | Focused DWG round trip; full gate pending. |
| Insert | Update-only | Transform transaction and undo tested; Python creation, deletion, real IPC/GUI and DWG/DXF gates pending. |
| Tolerance | **Complete** | `staged_python_tolerance_lifecycle_over_real_ipc` passes C/R/E/D/U/I/W/V/P with the staged API v7 plugin: high-level create/read/transaction edit/delete; insertion, direction, and text edits; live canvas selection; grouped undo/redo; invalid direction and missing style atomic rollback; exact untouched common/style/private-field preservation; and edited DWG plus DXF reopen. Host-owned coverage exposes placement, direction, normal, text, style name, height and gap; style handle is read-only and the undocumented DWG short remains unmapped. `tolerance_geometry_and_style_references_are_validated` and the existing Tolerance tessellation tests cover reference and rendered-frame behavior. |
| Shape | **Complete** | `staged_python_shape_lifecycle_over_real_ipc` passes C/R/E/D/U/I/W/V/P with a generated parser-valid SHX file and a real shape-file `TextStyle`: high-level create/read/transaction edit/delete; insertion, size, rotation and width-factor edits; live canvas selection; grouped undo/redo; invalid size and missing style rollback; exact identity/style/common-field preservation; and DWG plus DXF reopen. Each live and reopened document renders the three-point SHX glyph rather than the placeholder. Host-owned reference binding writes the stable style handle needed by DWG. The DXF fallback now searches all `is_shape_file` styles, preserving named-style glyph resolution after reopen. |
| AttributeDefinition | **Integration-tested / DXF blocker pending revalidation** | Host commit `5b854bd1` maps block ownership plus every stable ATTDEF field except the nested `embedded_mtext` payload, which is snapshot-readable and preserved untouched. `staged_python_attribute_definition_lifecycle_over_real_ipc` passes C/R/E/D/U/I/V/P with a real block record, Block/BlockEnd markers and linked Insert: high-level creation inside the block, readback, grouped placement/default/rotation/width edits, selection, rendered text, atomic tag/height/style/owner rejection, deletion persistence, and three-step undo/redo. DWG reopens with the full edited state. DXF reopens identity, ownership, tag, prompt, value, placement, height and rotation, and OCS no longer double-converts ATTDEF/ATTRIB rotation. **W remains open:** acadrust `568a12c` dropped optional ATTDEF fields including width factor while reading DXF. Rerun the regression against the currently merged `7ea4247` revision and update this row with the result. `embedded_mtext` mutation remains unsupported pending linked-layout validation. |
| AttributeEntity | **Complete** | OCS commit `cf2a9235` and adapter commit `9c6b5c3` make nested ATTRIB records addressable by handle while retaining canonical storage in `Insert.attributes`. Host validation binds tag to a definition in the Insert's block, validates placement/text geometry and style references, keeps owner and definition handles read-only, and preserves embedded MTEXT. `staged_python_attribute_entity_lifecycle_over_real_ipc` passes C/R/E/D/U/I/W/V/P with a real block, ATTDEF and Insert: high-level create/read/transaction edit/delete, parent selection, rendered text, atomic invalid tag/height/owner rejection, edited and deleted DWG/DXF reopen, and three-step undo/redo. `nested_insert_attribute_crud_uses_parent_storage_and_undo` additionally proves the child never becomes an orphan flat entity in the host document. |
| Hatch | **Complete** | OCS commit `98b56ef6` and adapter commit `a9b5de8` expose solid and patterned Hatches with elevation/normal, complete pattern definitions, pattern controls, style, association state, boundary paths and seed points. Nested tagged boundary edges use the reusable `{kind, value}` adapter form. Host validation requires closed finite loops, validates line/arc/ellipse/polyline/spline payloads and stored pattern lines, resolves associative boundary handles, and preserves gradient and MPOLYGON payloads as explicitly unsupported. `staged_python_hatch_lifecycle_over_real_ipc` passes C/R/E/D/U/I/W/V/P: Python solid creation, patterned boundary edit, live render-model pattern/boundary checks, selection, atomic invalid scale/association/open-loop/unmapped-property rejection, edited and deleted DWG/DXF reopen, post-reopen edits, and three-step undo/redo. |
| Leader | Layer-only | Requires annotation-linked fixture. |
| MLine | Layer-only | Requires valid style and vertex fixture. |
| Dimension | Layer-only | Track individual dimension subtypes. |
| MultiLeader | Layer-only | Requires nested context and style fixture. |
| Table | Layer-only | Requires table style and cell fixture. |
| PolygonMesh | Layer-only | Requires consistent grid/count fixture. |
| PolyfaceMesh | Layer-only | Requires valid face-index fixture. |
| Mesh | Layer-only | Requires valid topology fixture. |
| Helix | Layer-only | Requires valid derived-spline fixture. |
| RasterImage | Layer-only | Requires image definition and file fixture. |
| Wipeout | Layer-only | Requires clip and image-definition fixture. |
| Underlay | Layer-only | Requires external definition/file fixture. |
| Viewport | Layer-only | Requires layout and view-state fixture. |
| ViewBorder | Layer-only | Requires linked view records. |
| SectionSymbol | Layer-only | Requires section-view style and point records. |
| Light | Layer-only | Requires position/target and photometric-state fixture. |
| Region | Layer-only | Requires valid ACIS region payload. |
| Body | Layer-only | Requires valid ACIS body payload. |
| Solid3D | Layer-only | Requires valid ACIS solid payload. |
| Surface | Layer-only | Requires valid surface payload. |
| Ole2Frame | Layer-only | Requires valid embedded-object storage. |

After each increment, update the row with test names, fixture paths, exact
passing commands, the OCS commit hash, and any manual GUI result. Host and
adapter changes now share the same repository commit. If a gate is untested,
leave it pending. Do not raise the headline completion count from converter
coverage alone.
