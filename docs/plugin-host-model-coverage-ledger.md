# OCS canvas kind coverage ledger

Use with [the completion plan](plugin-host-model-completion-plan.md). The baseline below is conservative as of 19 September 2026. `Mapped` means at least one non-layer field is exposed for Python writes; it does **not** mean the kind passes the completion gate. `Update-only` means Python creation is unavailable. `Layer-only` means raw inspection and canvas layer edits exist, but geometry is not mapped. Record actual evidence or a blocking issue before changing any status to `Complete`.

Completion gate codes: **C** = create through Python document model; **R** = read; **E** = edit defining geometry and another property; **D** = delete; **U** = undo/redo and atomic failure; **I** = real IPC plus GUI/canvas pick or observable behavior; **W** = DWG and DXF save/reopen; **V** = invalid-input checks and untouched-field/reference preservation; **P** = portable H7 v5 build. A kind is `Complete` only when **C/R/E/D/U/I/W/V/P** all pass and its property catalog matches implementation. `Blocked` requires a specific dependency or failing fixture.

| Kind | Baseline | Gate evidence / blocker |
|---|---|---|
| Point | Mapped | Host transaction test now checks invalid batch rollback, grouped edit undo, and redo; creation/deletion history, real plugin execution, GUI, and DWG/DXF gates remain pending. |
| Line | Integration-tested | High-level document-model create/delete and transaction edit pass through the actual staged-plugin IPC runner; three undo entries, undo/redo, duplicate-handle atomic rejection, and edited DWG/DXF reopen pass in `staged_python_plugin_line_lifecycle_over_real_ipc`. GUI pick/visible-geometry observation and broader invalid-value/unmapped-field preservation checks remain pending; not complete. |
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
| Tolerance | Layer-only | Next new kind after integration harness. |
| Shape | Layer-only | Requires valid shape/style fixture. |
| AttributeDefinition | Layer-only | Requires block-definition fixture. |
| AttributeEntity | Layer-only | Requires linked Insert and attribute-sequence fixture. |
| Hatch | Layer-only | Requires valid boundary and pattern fixtures. |
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

After each increment, update the row with test names, fixture paths, exact passing commands, OCS and PandoraBox commit hashes, and any manual GUI result. If a gate is untested, leave it pending. Do not raise the headline completion count from converter coverage alone.
