# Scripting host model (API v7)

The host owns the drawing. Plugins can read the full `CadDocument` snapshot and
commit cloned `EntityType` values through IPC. `update_entities_transaction`
validates every replacement before changing the drawing, records one undo
entry, refreshes scene caches and the shared document view, and marks the
document dirty. The request preserves handles, owners, and entity kinds. A
missing or duplicate handle, a kind/owner change, or a locked source layer
rejects the whole batch. An empty batch does nothing.

This is a full-entity replacement API, not a promise that every CAD property
is editable through Python. A client should clone the current value and patch
only fields it understands; unknown fields remain intact. The host can read
all `acadrust::EntityType` variants through `document()`. Its generic write
path accepts existing variants subject to the validation above. Entity
creation and deletion still use `add_entity` and `remove_entity` and are not
part of this transaction request.

The host embeds a versioned coverage catalog generated from its traced
`EntityType` registry and `crates/ocs_plugin_api/entity_coverage_policy.json`.
It classifies all 48 variants: 43 canvas kinds, three internal records
(`Block`, `BlockEnd`, `Seqend`), and two opaque fallbacks (`Extended`,
`Unknown`). Each property records its source field, type, optional/sequence
shape, snapshot readability, Python access (`read_write`, `read_only`, or
`unmapped`), and validation status. `unmapped` means the typed snapshot carries
the field but the Python document model has no getter or setter for it.
The v7 transaction path checks changed Point locations, Line endpoints,
Circle/Arc centers, Ray/XLine base points, and Solid/Face3D corners for finite
coordinates. Circle/Arc radii must be finite and positive; Ray/XLine directions
must be unit vectors; Solid normals must be nonzero, its thickness finite,
and Face3D invisible-edge flags limited to four known bits. Insert transactions
validate placement, finite rotation and spacing, nonzero finite scale and normal,
and positive array counts. They reject changes to the referenced block and
attached attribute and sequence records. Those fields report `transaction_geometry` in the
catalog. All 43 canvas kinds also support nonempty `layer` changes through
the same undoable transaction; internal and opaque variants remain read-only
through the document model. The adapter accepts only `layer` patches for
canvas kinds outside its 38 geometry converters. Other writable properties
still report `type_conversion_only`; broader CAD range and cross-property
checks remain outstanding.
The host's `entity_snapshot` helper serializes any typed entity as a detached
JSON value keyed by its catalog variant name. This read path carries every
serializable field, including nested and unmapped values, without granting a
write path for them. The Python adapter exposes its variant payload through
`entity.snapshot`; changing that dictionary does not change the drawing.
It is an inspection view: JSON converts non-finite floating-point values to
null and cannot be used as a lossless replacement entity.

The bundled first-party adapter under `plugins/opencad-python` currently maps
these entity kinds to Python dictionaries: Point, Line, Circle, Arc, Ellipse,
Polyline, Polyline2D, Polyline3D, LwPolyline, Spline, Text, MText, Ray, XLine,
Solid, Face3D, Insert, Tolerance, Shape, AttributeDefinition,
AttributeEntity, and Hatch. An Insert is created by naming an existing
ordinary block (the host refuses unknown blocks, model or paper space and
containment cycles); the block name cannot be changed afterwards. Attached
attributes stay with the AttributeEntity flow and remain in the raw snapshot.
The legacy Polyline is update-only: create a Polyline2D or Polyline3D. The authoritative per-field mapping is
`plugins/opencad-python/entity_manifest.json`. The generated
mapping excludes fields it cannot represent, including common color, line
weight, transparency, Polyline3D smooth type, and MText background color.
Absent geometry fields are not editable through the Python document model.
A feature test compares the generated Python keys with the
host catalog. The adapter's `experimental-host-model` feature requires API
v7 and builds against the repository-relative `ocs_plugin_api`. RustPython is
loaded inside the separate plugin runner process; OCS does not link the Python
runtime into the editor executable.

API v7 also exposes a synchronous, ordered selection query and an atomic
selection replacement scoped to the session tab. All handles must exist before
replacement begins, duplicate handles are rejected, and the scripted
replacement keeps exactly the requested handles and order. UI picks continue
to group linked leaders and annotations. The V4 notification channel still broadcasts best-effort
document and selection changes. A new `DrawingChanged` notification carries
the scene epoch even when no shared document view is open. A new `CommandStateChanged` notification
reports transitions of the active interactive command at app message
boundaries, with the tab id and command name (or `None` when it ends). A
noninteractive command that starts and finishes within one message does not
produce an active-state transition.

The host's existing `InteractiveCommand` machinery collects either a point or
an entity pick. The bundled v7 adapter exposes a token-based request and
poll API because `PY_RUN` uses a fresh Python interpreter per invocation; a
script cannot suspend in place while the user clicks. Enter and command
cancellation report a cancelled result. The host releases the runner's pick
command when it ends, including Escape cancellation. Drawing, selection, and command
notifications are retained in a bounded 256-event queue per tab between runs
and can be drained with `doc.poll_events()`. If a tab exceeds that bound, the
next poll begins with `{"type": "overflow", "dropped": n}`; scripts should
refresh drawing and selection state when they receive it. Closing a tab clears
its pending events, and a script can poll only its active tab.
Input tokens are bound to the drawing tab that requested the pick. Polling a
token from another tab returns no result and does not consume it.

Python `doc.entities[handle]` returns a descriptor for every entity. For kinds
outside the 38-kind generated schema it contains only handle, kind, and layer;
`layer` is editable on canvas kinds. `doc.coverage()` enumerates the
catalog; `doc.coverage("Line")` and `line.coverage` return one entry. For
covered kinds, the coverage object lists actual readable and editable
dictionary keys, plus the unmapped snapshot fields and their types.
